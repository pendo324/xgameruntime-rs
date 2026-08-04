//! Blocking loopback TCP client to `xodus-service`, for the `XUser` methods that need a
//! real signed-in identity instead of an honest `E_NOTIMPL`.
//!
//! Blocking is deliberate, not a shortcut: `xasync.rs`'s `run_sync` executes its closure
//! synchronously on whatever thread called the `*Async` entry point (`XAsyncOp::Begin` in
//! `xasync_impl.rs` invokes it inline, before returning to the caller), and there is no
//! ambient tokio reactor anywhere in this crate for `tokio::net` to hook into. A loopback
//! round trip is expected to be fast, so blocking that one thread for it is architecturally
//! consistent with everything else this crate already does.
//!
//! This hand-mirrors two things from the sibling `xodus` workspace that this crate cannot
//! depend on directly (it is a separate, Windows-only crate cross-compiled for Wine):
//! - `xodus::ipc`: the env vars `xodus-cli run` sets on the game process
//!   (`ENV_TCP_PORT`/`ENV_TCP_SECRET`), and that the secret is hex-encoded on the wire.
//! - `xodus-service::connection`: the handshake (`tcp.rs`) and v2 XML framing
//!   (`mod.rs`/`xml.rs`) byte layouts. Only `Ping` and `MsaTokenRequest` have a working
//!   handler server-side today - everything else in `XodusMessageType` is schema-only.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use windows_core::HRESULT;

use crate::{E_FAIL, E_NOTIMPL};

const ENV_TCP_PORT: &str = "XODUS_TCP_PORT";
const ENV_TCP_SECRET: &str = "XODUS_TCP_SECRET";

/// ASCII "XDAU" on the wire. Distinct from the message magics so a client that skips the
/// handshake is rejected outright rather than read as a malformed secret.
const HANDSHAKE_MAGIC: u32 = 0x5541_4458;
const HANDSHAKE_ACCEPTED: u8 = 1;
const SECRET_LEN: usize = 32;

/// `XML_MAGIC_V2` from `xodus-service/src/main.rs`. The v2 framing (`u32` payload size)
/// is used unconditionally here rather than v1's `u16` - a gamer picture alone would
/// exceed v1's 64 KB ceiling, and there is no reason for a from-scratch client to take on
/// the smaller one.
const XML_MAGIC_V2: u32 = 0x5944_5358;

/// Mirrors `xodus-service::connection::MAX_MESSAGE_SIZE` - refuse to allocate for a
/// declared size the real service would never send, in case the port is somehow talking
/// to something else.
const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// `XodusMessageType::MSA_TOKEN_REQUEST` / `MSA_TOKEN_RESPONSE` (`proto/xodus/common.proto`).
/// Reused purely as a numeric message-type tag on the XML transport - the payload is XML,
/// not protobuf, `xml.rs::parse_message` just dispatches on this enum's discriminants.
const MSG_TYPE_MSA_TOKEN_REQUEST: u16 = 3;
const MSG_TYPE_MSA_TOKEN_RESPONSE: u16 = 4;
const MSG_TYPE_XSTS_TOKEN_REQUEST: u16 = 5;
const MSG_TYPE_XSTS_TOKEN_RESPONSE: u16 = 6;
const MSG_TYPE_USER_INFO_REQUEST: u16 = 7;
const MSG_TYPE_USER_INFO_RESPONSE: u16 = 8;

/// Xbox Live's own MSA app registration id, used throughout `xodus`'s auth flow
/// (`xodus::auth::TitleIdentity::default`) - not a per-title/per-game id, so hardcoding it
/// here is not the "don't hardcode the title identity" case PLAN.md warns about.
const XBOX_LIVE_CLIENT_ID: &str = "000000004424da1f";

/// The literal scope GDK games pass to request a full-trust (`MBI_SSL`) token, per
/// `xodus-service/src/connection/xml.rs`'s handling of `MSATokenRequest::msa_full_trust`.
/// Anything else is treated as an ordinary sign-in scope request.
const FULL_TRUST_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";

// Both structs derive both directions: production code only ever serializes a request
// and deserializes a response, but the test below plays xodus-service's part of the
// conversation (deserializing the request it received, serializing the response it
// sends back) to exercise the wire format without a real service running.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename = "MSATokenRequest")]
#[serde(rename_all = "PascalCase")]
struct MsaTokenRequest {
    client_id: String,
    #[cfg_attr(not(test), allow(dead_code))]
    allow_ui: bool,
    msa_full_trust: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MsaTokenResponse {
    token: String,
    expiry: i64,
    #[allow(dead_code)]
    device_rps: String,
    #[allow(dead_code)]
    device_expiry: i64,
}

/// `xodus-service`'s `XstsTokenRequest` handler derives the relying party itself from
/// `url` (Xbox Live's title-management endpoint table), the same way the real title-managed
/// SDK would - the caller never supplies one, matching `XUserGetTokenAndSignatureAsync`'s
/// real signature.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XstsTokenRequest {
    method: String,
    url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    force_refresh: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XstsTokenResponse {
    #[allow(dead_code)]
    token: String,
    authorization: String,
    #[serde(default)]
    signature: String,
    #[allow(dead_code)]
    expiry: i64,
}

/// No request fields - `xodus-service` always answers for whichever user's credentials
/// are on this connection.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserInfoRequest {}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserInfoResponse {
    xuid: String,
    gamertag: String,
    #[serde(default)]
    gamertag_modern: String,
    age_group: String,
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard (padded) base64 - `body` is an arbitrary byte buffer riding inside XML text,
/// which cannot carry raw bytes safely.
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(BASE64_ALPHABET[(n >> 18 & 0x3f) as usize] as char);
        out.push(BASE64_ALPHABET[(n >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(n >> 6 & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Where to find `xodus-service`, as published to the game process's environment by
/// `xodus-cli run`. `Err(E_NOTIMPL)` here means "not running under `xodus-cli run`" - a
/// distinct, honest condition from a service that is reachable but refused the connection.
fn endpoint() -> Result<(u16, Vec<u8>), HRESULT> {
    let port: u16 = std::env::var(ENV_TCP_PORT)
        .ok()
        .and_then(|p| p.parse().ok())
        .ok_or(E_NOTIMPL)?;
    let secret = std::env::var(ENV_TCP_SECRET)
        .ok()
        .and_then(|s| hex_decode(&s))
        .filter(|s| s.len() == SECRET_LEN)
        .ok_or(E_NOTIMPL)?;
    Ok((port, secret))
}

fn perform_handshake(stream: &mut TcpStream, secret: &[u8]) -> Result<(), HRESULT> {
    stream
        .write_all(&HANDSHAKE_MAGIC.to_le_bytes())
        .map_err(|_| E_FAIL)?;
    stream.write_all(secret).map_err(|_| E_FAIL)?;

    let mut accepted = [0u8; 1];
    stream.read_exact(&mut accepted).map_err(|_| E_FAIL)?;
    if accepted[0] != HANDSHAKE_ACCEPTED {
        return Err(E_FAIL);
    }
    Ok(())
}

/// One request/response round trip: connect, handshake, send one v2-framed XML message,
/// read one back. `xodus-service` serves one message per accepted connection loop
/// iteration but happily keeps reading more on the same connection, so a fresh connection
/// per call is not required by the protocol - it is just simpler, and loopback connection
/// setup is cheap next to the token exchange this exists to make.
fn request(msg_type: u16, payload: &[u8]) -> Result<(u16, Vec<u8>), HRESULT> {
    let (port, secret) = endpoint()?;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|_| E_FAIL)?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_nodelay(true).ok();

    perform_handshake(&mut stream, &secret)?;

    let mut request = Vec::with_capacity(payload.len() + 10);
    request.extend(XML_MAGIC_V2.to_le_bytes());
    request.extend(msg_type.to_le_bytes());
    request.extend((payload.len() as u32).to_le_bytes());
    request.extend_from_slice(payload);
    stream.write_all(&request).map_err(|_| E_FAIL)?;

    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).map_err(|_| E_FAIL)?;
    if u32::from_le_bytes(magic) != XML_MAGIC_V2 {
        return Err(E_FAIL);
    }

    let mut header = [0u8; 6];
    stream.read_exact(&mut header).map_err(|_| E_FAIL)?;
    let reply_type = u16::from_le_bytes([header[0], header[1]]);
    let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
    if size > MAX_MESSAGE_SIZE {
        return Err(E_FAIL);
    }

    let mut body = vec![0u8; size];
    stream.read_exact(&mut body).map_err(|_| E_FAIL)?;

    Ok((reply_type, body))
}

/// `XUserGetMsaTokenSilentlyAsync`'s real backing. `scope` is the raw string the game
/// passed in; anything other than [`FULL_TRUST_SCOPE`] is treated as an ordinary sign-in
/// scope request. Returns `(token, expiry_unix_seconds)`.
pub fn get_msa_token_silently(scope: Option<&str>) -> Result<(String, i64), HRESULT> {
    let request_body = MsaTokenRequest {
        client_id: XBOX_LIVE_CLIENT_ID.to_string(),
        allow_ui: false,
        msa_full_trust: scope == Some(FULL_TRUST_SCOPE),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_MSA_TOKEN_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_MSA_TOKEN_RESPONSE {
        // xml.rs::parse_message answers every unhandled message type (including a
        // malformed request that failed to deserialize) with an empty buffer at
        // `request_type + 1`, so this also covers "the service didn't understand us".
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: MsaTokenResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok((response.token, response.expiry))
}

/// `XUserGetTokenAndSignatureAsync`'s real backing. Returns `(authorization_header,
/// signature_header)` - `signature_header` is empty when no signature policy covers `url`
/// or no device proof key has been provisioned yet (`xodus-cli device-auth`), matching real
/// GDK behavior for endpoints that don't require request signing.
pub fn get_token_and_signature(
    method: &str,
    url: &str,
    body: &[u8],
) -> Result<(String, String), HRESULT> {
    let request_body = XstsTokenRequest {
        method: method.to_string(),
        url: url.to_string(),
        body: base64_encode(body),
        force_refresh: false,
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_XSTS_TOKEN_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_XSTS_TOKEN_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: XstsTokenResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok((response.authorization, response.signature))
}

/// `XUserAddAsync`'s silent path, plus `XUserGetGamertag`/`XUserGetAgeGroup`'s backing data -
/// GDK caches these on the `XUserHandle` at sign-in rather than re-fetching per call, so
/// callers should do the same. Returns `(xuid, gamertag, gamertag_modern, age_group)`; `age_group`
/// is Xbox Live's raw claim (`"Adult"`/`"Teen"`/`"Child"`), not yet mapped to `XUserAgeGroup`.
pub fn get_user_info() -> Result<(String, String, String, String), HRESULT> {
    let body = quick_xml::se::to_string(&UserInfoRequest {}).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_USER_INFO_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_USER_INFO_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: UserInfoResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok((
        response.xuid,
        response.gamertag,
        response.gamertag_modern,
        response.age_group,
    ))
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::Mutex;

    use super::*;

    // `get_msa_token_silently` reads `ENV_TCP_PORT`/`ENV_TCP_SECRET`, which are
    // process-global - serialize the tests that touch them so they don't race each other
    // under `cargo test`'s default parallelism.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    fn set_endpoint_env(port: u16, secret_hex: &str) {
        // SAFETY: serialized by ENV_GUARD, and this crate is a `cdylib`/`rlib` test binary
        // with no other threads reading these vars concurrently.
        unsafe {
            std::env::set_var(ENV_TCP_PORT, port.to_string());
            std::env::set_var(ENV_TCP_SECRET, secret_hex);
        }
    }

    fn clear_endpoint_env() {
        unsafe {
            std::env::remove_var(ENV_TCP_PORT);
            std::env::remove_var(ENV_TCP_SECRET);
        }
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn read_exact_blocking(stream: &mut std::net::TcpStream, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        stream.read_exact(&mut buf).expect("read");
        buf
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn hex_decode_round_trips_and_rejects_garbage() {
        let bytes: Vec<u8> = (0..32).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)), Some(bytes));
        assert_eq!(hex_decode("abc"), None); // odd length
        assert_eq!(hex_decode("zz"), None); // not hex
    }

    #[test]
    fn missing_env_is_reported_as_not_implemented_not_a_crash() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        clear_endpoint_env();
        assert_eq!(endpoint().unwrap_err(), E_NOTIMPL);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let real_secret = [0x11u8; SECRET_LEN];

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut magic = [0u8; 4];
            socket.read_exact(&mut magic).expect("read magic");
            assert_eq!(u32::from_le_bytes(magic), HANDSHAKE_MAGIC);
            let mut presented = [0u8; SECRET_LEN];
            socket.read_exact(&mut presented).expect("read secret");
            // Real xodus-service closes the connection with no reply on a bad secret.
            assert_ne!(presented, real_secret);
        });

        set_endpoint_env(port, &hex_encode(&[0x22u8; SECRET_LEN]));
        let result = get_msa_token_silently(None);
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(result.unwrap_err(), E_FAIL);
    }

    #[test]
    fn msa_token_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x42u8; SECRET_LEN];
        let secret_for_server = secret;

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");

            let mut magic = [0u8; 4];
            socket.read_exact(&mut magic).expect("read handshake magic");
            assert_eq!(u32::from_le_bytes(magic), HANDSHAKE_MAGIC);
            let presented = read_exact_blocking(&mut socket, SECRET_LEN);
            assert_eq!(presented, secret_for_server);
            socket
                .write_all(&[HANDSHAKE_ACCEPTED])
                .expect("write accepted");

            let mut msg_magic = [0u8; 4];
            socket
                .read_exact(&mut msg_magic)
                .expect("read message magic");
            assert_eq!(u32::from_le_bytes(msg_magic), XML_MAGIC_V2);
            let mut header = [0u8; 6];
            socket.read_exact(&mut header).expect("read header");
            let msg_type = u16::from_le_bytes([header[0], header[1]]);
            assert_eq!(msg_type, MSG_TYPE_MSA_TOKEN_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: MsaTokenRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.client_id, XBOX_LIVE_CLIENT_ID);
            assert!(!request.msa_full_trust);

            let response = MsaTokenResponse {
                token: "fake-user-token".to_string(),
                expiry: 1_700_000_000,
                device_rps: String::new(),
                device_expiry: 0,
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_MSA_TOKEN_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_msa_token_silently(None);
        clear_endpoint_env();
        server.join().expect("server thread");

        let (token, expiry) = result.expect("round trip succeeds");
        assert_eq!(token, "fake-user-token");
        assert_eq!(expiry, 1_700_000_000);
    }

    #[test]
    fn xsts_token_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x77u8; SECRET_LEN];
        let secret_for_server = secret;

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");

            let mut magic = [0u8; 4];
            socket.read_exact(&mut magic).expect("read handshake magic");
            assert_eq!(u32::from_le_bytes(magic), HANDSHAKE_MAGIC);
            let presented = read_exact_blocking(&mut socket, SECRET_LEN);
            assert_eq!(presented, secret_for_server);
            socket
                .write_all(&[HANDSHAKE_ACCEPTED])
                .expect("write accepted");

            let mut msg_magic = [0u8; 4];
            socket
                .read_exact(&mut msg_magic)
                .expect("read message magic");
            assert_eq!(u32::from_le_bytes(msg_magic), XML_MAGIC_V2);
            let mut header = [0u8; 6];
            socket.read_exact(&mut header).expect("read header");
            let msg_type = u16::from_le_bytes([header[0], header[1]]);
            assert_eq!(msg_type, MSG_TYPE_XSTS_TOKEN_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: XstsTokenRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, "https://profile.xboxlive.com/users/me");
            assert_eq!(request.body, base64_encode(b"payload"));

            let response = XstsTokenResponse {
                token: "fake-xsts-token".to_string(),
                authorization: "XBL3.0 x=fake-uhs;fake-xsts-token".to_string(),
                signature: "fake-signature".to_string(),
                expiry: 1_700_000_000,
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_XSTS_TOKEN_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result =
            get_token_and_signature("GET", "https://profile.xboxlive.com/users/me", b"payload");
        clear_endpoint_env();
        server.join().expect("server thread");

        let (authorization, signature) = result.expect("round trip succeeds");
        assert_eq!(authorization, "XBL3.0 x=fake-uhs;fake-xsts-token");
        assert_eq!(signature, "fake-signature");
    }

    #[test]
    fn user_info_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x99u8; SECRET_LEN];
        let secret_for_server = secret;

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");

            let mut magic = [0u8; 4];
            socket.read_exact(&mut magic).expect("read handshake magic");
            assert_eq!(u32::from_le_bytes(magic), HANDSHAKE_MAGIC);
            let presented = read_exact_blocking(&mut socket, SECRET_LEN);
            assert_eq!(presented, secret_for_server);
            socket
                .write_all(&[HANDSHAKE_ACCEPTED])
                .expect("write accepted");

            let mut msg_magic = [0u8; 4];
            socket
                .read_exact(&mut msg_magic)
                .expect("read message magic");
            assert_eq!(u32::from_le_bytes(msg_magic), XML_MAGIC_V2);
            let mut header = [0u8; 6];
            socket.read_exact(&mut header).expect("read header");
            let msg_type = u16::from_le_bytes([header[0], header[1]]);
            assert_eq!(msg_type, MSG_TYPE_USER_INFO_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let _request: UserInfoRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");

            let response = UserInfoResponse {
                xuid: "2533274999999999".to_string(),
                gamertag: "FakeGamer".to_string(),
                gamertag_modern: "FakeGamer".to_string(),
                age_group: "Adult".to_string(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_USER_INFO_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_user_info();
        clear_endpoint_env();
        server.join().expect("server thread");

        let (xuid, gamertag, gamertag_modern, age_group) = result.expect("round trip succeeds");
        assert_eq!(xuid, "2533274999999999");
        assert_eq!(gamertag, "FakeGamer");
        assert_eq!(gamertag_modern, "FakeGamer");
        assert_eq!(age_group, "Adult");
    }
}
