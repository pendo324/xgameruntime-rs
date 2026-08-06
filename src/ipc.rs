//! Blocking loopback TCP client to `xodus-service`, for the `XUser` methods that need a
//! real signed-in identity instead of an `E_NOTIMPL`.
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
//!   (`mod.rs`/`xml.rs`) byte layouts. `Ping`, `MSATokenRequest`, `XstsTokenRequest`,
//!   `UserInfoRequest`, and `LicenseRequest` have working handlers server-side today -
//!   everything else in `XodusMessageType` is schema-only.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use windows_core::HRESULT;

use crate::diag::diag;
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

/// Interactive sign-in blocks on a human finishing (or abandoning) a webview login, which
/// runs on human timescale, not the sub-second round trips [`IO_TIMEOUT`] is sized for. Ten
/// minutes is generous enough not to cut off a real attempt while still eventually giving
/// up if the webview process wedges.
const INTERACTIVE_SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);

/// `XodusMessageType::MSA_TOKEN_REQUEST` / `MSA_TOKEN_RESPONSE` (`proto/xodus/common.proto`).
/// Reused purely as a numeric message-type tag on the XML transport - the payload is XML,
/// not protobuf, `xml.rs::parse_message` just dispatches on this enum's discriminants.
const MSG_TYPE_MSA_TOKEN_REQUEST: u16 = 3;
const MSG_TYPE_MSA_TOKEN_RESPONSE: u16 = 4;
const MSG_TYPE_XSTS_TOKEN_REQUEST: u16 = 5;
const MSG_TYPE_XSTS_TOKEN_RESPONSE: u16 = 6;
const MSG_TYPE_USER_INFO_REQUEST: u16 = 7;
const MSG_TYPE_USER_INFO_RESPONSE: u16 = 8;
const MSG_TYPE_LICENSE_REQUEST: u16 = 9;
const MSG_TYPE_LICENSE_RESPONSE: u16 = 10;
const MSG_TYPE_ENTITLED_PRODUCTS_REQUEST: u16 = 13;
const MSG_TYPE_ENTITLED_PRODUCTS_RESPONSE: u16 = 14;
const MSG_TYPE_COLLECTIONS_ID_REQUEST: u16 = 15;
const MSG_TYPE_COLLECTIONS_ID_RESPONSE: u16 = 16;
const MSG_TYPE_LICENSE_TOKEN_REQUEST: u16 = 17;
const MSG_TYPE_LICENSE_TOKEN_RESPONSE: u16 = 18;
const MSG_TYPE_ASSOCIATED_PRODUCTS_REQUEST: u16 = 19;
const MSG_TYPE_ASSOCIATED_PRODUCTS_RESPONSE: u16 = 20;
const MSG_TYPE_RESOLVE_PRODUCT_ID_REQUEST: u16 = 21;
const MSG_TYPE_RESOLVE_PRODUCT_ID_RESPONSE: u16 = 22;
const MSG_TYPE_INTERACTIVE_SIGN_IN_REQUEST: u16 = 23;
const MSG_TYPE_INTERACTIVE_SIGN_IN_RESPONSE: u16 = 24;
const MSG_TYPE_GAMER_PICTURE_REQUEST: u16 = 25;
const MSG_TYPE_GAMER_PICTURE_RESPONSE: u16 = 26;
/// Mirrors `xodus_service::connection::xml::ERROR_REPLY_TYPE`: sent instead of
/// `msg_type + 1` when the service hit an internal error handling the request (e.g. a
/// transient failure talking to a real Microsoft endpoint), with the error's `Display`
/// text as the body. Distinguishes "the service failed" from "legitimately empty success",
/// which used to both show up on the wire as an empty body at `msg_type + 1`.
const MSG_TYPE_ERROR: u16 = 0xFFFF;

/// `xodus-cli run` publishes the launched package's `ContentId` here (`xodus::ipc::ENV_CONTENT_ID`
/// on the `xodus` side - this crate can't depend on that crate, so the literal is hand-mirrored,
/// same as [`ENV_TCP_PORT`]/[`ENV_TCP_SECRET`]). Unset when not running under `xodus-cli run`.
const ENV_CONTENT_ID: &str = "XODUS_CONTENT_ID";

/// `xodus-cli run` publishes the launched package's computed `PackageFamilyName` here
/// (`xodus::ipc::ENV_PACKAGE_FAMILY_NAME` on the `xodus` side, hand-mirrored for the same
/// reason as [`ENV_CONTENT_ID`]). Unset when not running under `xodus-cli run`, or when
/// `xodus-cli run` couldn't find/parse an `AppxManifest.xml`.
pub(crate) const ENV_PACKAGE_FAMILY_NAME: &str = "XODUS_PACKAGE_FAMILY_NAME";

/// `xodus-cli run`'s parse of the launched package's `<PersistentLocalStorage>` element from
/// `MicrosoftGame.config` (`xodus::ipc::ENV_PLS_*` on the `xodus` side, hand-mirrored for the
/// same reason as [`ENV_CONTENT_ID`]). `ENV_PLS_SHAREABLE` unset means no such element was
/// found - `XPersistentLocalStorageGetSpaceInfo` falls back to a placeholder in that case.
const ENV_PLS_SIZE_MB: &str = "XODUS_PLS_SIZE_MB";
const ENV_PLS_GROWABLE_TO_MB: &str = "XODUS_PLS_GROWABLE_TO_MB";
const ENV_PLS_SHAREABLE: &str = "XODUS_PLS_SHAREABLE";

/// Comma-separated `StoreId`s from the launched package's `<RelatedProducts>` declaration
/// (`xodus::ipc::ENV_RELATED_PRODUCTS` on the `xodus` side) - the products this title allows
/// `XPersistentLocalStorageMountForPackage` to mount storage for.
const ENV_RELATED_PRODUCTS: &str = "XODUS_RELATED_PRODUCTS";

/// A `Z:\...`-rooted path to a real, persistent (survives reboots) per-title directory
/// `xodus-cli run` created under a host XDG data dir (`xodus::ipc::ENV_GAME_SAVE_ROOT` on the
/// `xodus` side, hand-mirrored for the same reason as [`ENV_CONTENT_ID`]) - `XGameSave`'s local
/// container store lives under here. Unset when not running under `xodus-cli run`, or when it
/// couldn't resolve a `PackageFamilyName`/create the directory.
const ENV_GAME_SAVE_ROOT: &str = "XODUS_GAME_SAVE_ROOT";

/// Xbox Live's own MSA app registration id, used throughout `xodus`'s auth flow
/// (`xodus::auth::TitleIdentity::xodus`) - not a per-title/per-game id. Used only as a
/// fallback by [`title_client_id`] when the launched title has no `MicrosoftGame.config`
/// (or no `<MSAAppId>` in it) to read a real one from - safe to hardcode because it's
/// shared Xbox Live infrastructure, not any particular game's identity.
const XBOX_LIVE_CLIENT_ID: &str = "000000004424da1f";

/// Parses the real `<MSAAppId>` out of the launched title's `MicrosoftGame.config`, walking
/// up from the game executable the same way `com.rs`'s `read_game_title_id` does for
/// `<TitleId>` (the file lives next to the exe, occasionally a parent directory) - not
/// hardcoded, so the app id is the launched title's own rather than baked in. `None` when no
/// `MicrosoftGame.config`/`<MSAAppId>` was found, so callers fall back to
/// [`XBOX_LIVE_CLIENT_ID`].
fn read_game_msa_app_id() -> Option<String> {
    static MSA_APP_ID: OnceLock<Option<String>> = OnceLock::new();
    MSA_APP_ID
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let mut dir = exe.parent()?.to_path_buf();
            loop {
                for name in ["MicrosoftGame.Config", "MicrosoftGame.config"] {
                    let candidate = dir.join(name);
                    if let Ok(contents) = std::fs::read_to_string(&candidate)
                        && let Some(id) = parse_msa_app_id_from_config(&contents)
                    {
                        return Some(id);
                    }
                }
                if !dir.pop() {
                    return None;
                }
            }
        })
        .clone()
}

fn parse_msa_app_id_from_config(contents: &str) -> Option<String> {
    let start = contents.find("<MSAAppId")?;
    let open_end = contents[start..].find('>')? + start + 1;
    let close = contents[open_end..].find("</MSAAppId>")? + open_end;
    let text = contents[open_end..close].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// The `client_id` to use for MSA/Xbox Live token exchanges: the launched title's own real
/// `<MSAAppId>` when one could be read, falling back to the shared [`XBOX_LIVE_CLIENT_ID`]
/// otherwise - never a hardcoded per-title value.
fn title_client_id() -> String {
    read_game_msa_app_id().unwrap_or_else(|| XBOX_LIVE_CLIENT_ID.to_string())
}

/// The literal scope GDK games pass to request a full-trust (`MBI_SSL`) token, per
/// `xodus-service/src/connection/xml.rs`'s handling of `MSATokenRequest::msa_full_trust`.
/// Anything else is treated as an ordinary sign-in scope request.
const FULL_TRUST_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";

// Request/response shapes for xodus-service's loopback IPC. The authoritative
// definitions live in `xodus-ipc-models` (a sub-crate of the `xodus` repo) - the single
// source of truth both this DLL and xodus-service serialize against, so the two
// hand-mirrored copies can't drift apart. Names here follow the crate's canonical
// spelling (`MSATokenRequest`, `AssociatedProductEntry`, ...).
use xodus_ipc_models::xstore::{
    AssociatedProductEntry, AssociatedProductsRequest, AssociatedProductsResponse,
    CollectionsIdRequest, CollectionsIdResponse, EntitledProduct, EntitledProductsRequest,
    EntitledProductsResponse, LicenseRequest, LicenseResponse, LicenseTokenRequest,
    LicenseTokenResponse, ResolveProductIdRequest, ResolveProductIdResponse,
};
use xodus_ipc_models::xuser::{
    GamerPictureRequest, GamerPictureResponse, InteractiveSignInRequest, InteractiveSignInResponse,
    MSATokenRequest, MSATokenResponse, UserInfoRequest, UserInfoResponse, XstsTokenRequest,
    XstsTokenResponse,
};

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

/// Inverse of [`base64_encode`] - standard (padded) alphabet only, matching what
/// `xodus-service` sends back. Returns `None` on malformed input rather than a partial
/// buffer.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        BASE64_ALPHABET
            .iter()
            .position(|&b| b == c)
            .map(|i| i as u32)
    }

    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let chars: Vec<u8> = s.bytes().collect();
    for chunk in chars.chunks(4) {
        if chunk.len() < 2 {
            return None;
        }
        let mut n: u32 = 0;
        for &c in chunk {
            n = (n << 6) | value(c)?;
        }
        n <<= 6 * (4 - chunk.len() as u32);
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Where to find `xodus-service`, as published to the game process's environment by
/// `xodus-cli run`. `Err(E_NOTIMPL)` here means "not running under `xodus-cli run`" - a
/// distinct condition from a service that is reachable but refused the connection.
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
///
/// Reply bodies carry live Xbox Live credentials (XSTS `XBL3.0` headers, MSA compact
/// tokens, request signatures). Never log one: log sizes and outcomes instead, the way
/// the callers below do.
fn request(msg_type: u16, payload: &[u8]) -> Result<(u16, Vec<u8>), HRESULT> {
    request_with_timeout(msg_type, payload, IO_TIMEOUT)
}

/// Same round trip as [`request`], but with a caller-chosen read/write timeout - for calls
/// like [`interactive_sign_in`] that wait on human timescale, not [`IO_TIMEOUT`]'s
/// sub-second round-trip assumption.
fn request_with_timeout(
    msg_type: u16,
    payload: &[u8],
    io_timeout: Duration,
) -> Result<(u16, Vec<u8>), HRESULT> {
    diag!("request msg_type={msg_type} starting");
    let (port, secret) = endpoint()
        .inspect_err(|e| diag!("request msg_type={msg_type} endpoint() failed: {e:?}"))?;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| {
        diag!("request msg_type={msg_type} connect to {addr} failed: {e}");
        E_FAIL
    })?;
    stream.set_read_timeout(Some(io_timeout)).ok();
    stream.set_write_timeout(Some(io_timeout)).ok();
    stream.set_nodelay(true).ok();

    perform_handshake(&mut stream, &secret)
        .inspect_err(|e| diag!("request msg_type={msg_type} handshake failed: {e:?}"))?;

    let mut request = Vec::with_capacity(payload.len() + 10);
    request.extend(XML_MAGIC_V2.to_le_bytes());
    request.extend(msg_type.to_le_bytes());
    request.extend((payload.len() as u32).to_le_bytes());
    request.extend_from_slice(payload);
    stream.write_all(&request).map_err(|e| {
        diag!("request msg_type={msg_type} write failed: {e}");
        E_FAIL
    })?;

    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).map_err(|e| {
        diag!("request msg_type={msg_type} read magic failed: {e}");
        E_FAIL
    })?;
    if u32::from_le_bytes(magic) != XML_MAGIC_V2 {
        diag!("request msg_type={msg_type} bad reply magic: {magic:?}");
        return Err(E_FAIL);
    }

    let mut header = [0u8; 6];
    stream.read_exact(&mut header).map_err(|e| {
        diag!("request msg_type={msg_type} read header failed: {e}");
        E_FAIL
    })?;
    let reply_type = u16::from_le_bytes([header[0], header[1]]);
    let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
    if size > MAX_MESSAGE_SIZE {
        diag!("request msg_type={msg_type} reply size {size} too large");
        return Err(E_FAIL);
    }

    let mut body = vec![0u8; size];
    stream.read_exact(&mut body).map_err(|e| {
        diag!("request msg_type={msg_type} read body failed: {e}");
        E_FAIL
    })?;

    diag!("request msg_type={msg_type} succeeded, reply_type={reply_type} size={size}");
    if reply_type == MSG_TYPE_ERROR {
        diag!(
            "request msg_type={msg_type} server reported an error: {}",
            String::from_utf8_lossy(&body)
        );
    }
    Ok((reply_type, body))
}

/// `XUserGetMsaTokenSilentlyAsync`'s real backing. `scope` is the raw string the game
/// passed in; anything other than [`FULL_TRUST_SCOPE`] is treated as an ordinary sign-in
/// scope request. Returns `(token, expiry_unix_seconds)`.
pub fn get_msa_token_silently(scope: Option<&str>) -> Result<(String, i64), HRESULT> {
    let request_body = MSATokenRequest {
        client_id: title_client_id(),
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
    let response: MSATokenResponse = match quick_xml::de::from_str(text) {
        Ok(r) => r,
        Err(err) => {
            diag!("get_msa_token_silently deserialize error: {err}");
            return Err(E_FAIL);
        }
    };
    diag!(
        "get_msa_token_silently -> token ({} bytes) expiry={}",
        response.token.len(),
        response.expiry
    );
    Ok((response.token, response.expiry))
}

/// How many times [`get_token_and_signature`] retries a failed round trip before giving up.
/// Observed in practice: the very first XSTS token-and-signature fetch for a relying party
/// can fail transiently (a retry moments later against the same relying party succeeds with
/// a normal token), and Bedrock does not tolerate that failure gracefully - it falls back to
/// showing the sign-in prompt even though a valid signed-in user handle already exists. This
/// mirrors how real Xbox Live clients (e.g. `xbox-live-api`) treat token-and-signature
/// fetches as retryable rather than fatal on the first failure.
const TOKEN_AND_SIGNATURE_RETRIES: u32 = 3;
const TOKEN_AND_SIGNATURE_RETRY_DELAY: Duration = Duration::from_millis(300);

/// `XUserGetTokenAndSignatureAsync`'s real backing. Returns `(authorization_header,
/// signature_header)` - `signature_header` is empty when no signature policy covers `url`
/// or no device proof key has been provisioned yet (`xodus-cli device-auth`), matching real
/// GDK behavior for endpoints that don't require request signing.
pub fn get_token_and_signature(
    method: &str,
    url: &str,
    body: &[u8],
) -> Result<(String, String), HRESULT> {
    let mut last_err = E_FAIL;
    for attempt in 1..=TOKEN_AND_SIGNATURE_RETRIES {
        match get_token_and_signature_once(method, url, body) {
            Ok(result) => return Ok(result),
            Err(err) => {
                diag!(
                    "get_token_and_signature({url}) attempt {attempt}/{TOKEN_AND_SIGNATURE_RETRIES} failed: {err:?}"
                );
                last_err = err;
                if attempt < TOKEN_AND_SIGNATURE_RETRIES {
                    std::thread::sleep(TOKEN_AND_SIGNATURE_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_err)
}

fn get_token_and_signature_once(
    method: &str,
    url: &str,
    body: &[u8],
) -> Result<(String, String), HRESULT> {
    let request_body = XstsTokenRequest {
        method: method.to_string(),
        url: url.to_string(),
        body: base64_encode(body),
        force_refresh: false,
        client_id: title_client_id(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_XSTS_TOKEN_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_XSTS_TOKEN_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: XstsTokenResponse = match quick_xml::de::from_str(text) {
        Ok(r) => r,
        Err(err) => {
            diag!("get_token_and_signature deserialize error: {err}");
            return Err(E_FAIL);
        }
    };
    diag!(
        "get_token_and_signature({url}) -> authorization ({} bytes) signature ({} bytes)",
        response.authorization.len(),
        response.signature.len()
    );
    Ok((response.authorization, response.signature))
}

/// `XUserAddAsync`'s silent path, plus `XUserGetGamertag`/`XUserGetAgeGroup`'s backing data -
/// GDK caches these on the `XUserHandle` at sign-in rather than re-fetching per call, so
/// callers should do the same. Returns `(xuid, gamertag, gamertag_modern, age_group)`; `age_group`
/// is Xbox Live's raw claim (`"Adult"`/`"Teen"`/`"Child"`), not yet mapped to `XUserAgeGroup`.
pub fn get_user_info() -> Result<(String, String, String, String), HRESULT> {
    let body = quick_xml::se::to_string(&UserInfoRequest {
        client_id: title_client_id(),
    })
    .map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_USER_INFO_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_USER_INFO_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: UserInfoResponse = match quick_xml::de::from_str(text) {
        Ok(r) => r,
        Err(err) => {
            diag!("get_user_info deserialize error: {err}");
            return Err(E_FAIL);
        }
    };
    diag!(
        "get_user_info parsed: xuid={:?} gamertag={:?} gamertag_modern={:?} age_group={:?}",
        response.xuid,
        response.gamertag,
        response.gamertag_modern,
        response.age_group
    );
    Ok((
        response.xuid,
        response.gamertag,
        response.gamertag_modern,
        response.age_group,
    ))
}

/// `XUserAddAsync(AddDefaultUserAllowingUI)` / `XUserAddByIdWithUiAsync`'s real backing when
/// there is nobody signed in for [`get_user_info`] to answer silently. Blocks for as long as
/// `xodus-service`'s spawned `xodus-cli login` webview takes - a human deciding whether to
/// sign in, not a network round trip, hence [`INTERACTIVE_SIGN_IN_TIMEOUT`] rather than the
/// default. Returns `Ok(None)` when the human closed the window without completing sign-in
/// (a "declined", not an error); `Ok(Some(..))` on the same
/// `(xuid, gamertag, gamertag_modern, age_group)` shape as [`get_user_info`] on success.
pub fn interactive_sign_in() -> Result<Option<(String, String, String, String)>, HRESULT> {
    diag!("interactive_sign_in called");
    let body = quick_xml::se::to_string(&InteractiveSignInRequest {
        client_id: title_client_id(),
    })
    .map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request_with_timeout(
        MSG_TYPE_INTERACTIVE_SIGN_IN_REQUEST,
        body.as_bytes(),
        INTERACTIVE_SIGN_IN_TIMEOUT,
    )?;
    if reply_type != MSG_TYPE_INTERACTIVE_SIGN_IN_RESPONSE {
        diag!("interactive_sign_in unexpected reply_type={reply_type}");
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: InteractiveSignInResponse = match quick_xml::de::from_str(text) {
        Ok(r) => r,
        Err(err) => {
            diag!("interactive_sign_in deserialize error: {err}");
            return Err(E_FAIL);
        }
    };
    if !response.success {
        diag!("interactive_sign_in: server reported declined/failed sign-in");
        return Ok(None);
    }
    Ok(Some((
        response.xuid,
        response.gamertag,
        response.gamertag_modern,
        response.age_group,
    )))
}

/// `XStoreQueryGameLicenseAsync`'s real backing. Returns `(is_active, expiration_date)` for
/// the package `xodus-cli run` launched (identified by its `ContentId`, published via
/// [`ENV_CONTENT_ID`]). `Err(E_NOTIMPL)` when that env var is unset - not running under
/// `xodus-cli run`, so there is no package to ask about - distinct from a reachable service
/// that actually answered "not entitled".
pub fn get_game_license() -> Result<(bool, i64), HRESULT> {
    let content_id = std::env::var(ENV_CONTENT_ID).map_err(|_| E_NOTIMPL)?;

    let request_body = LicenseRequest {
        content_id,
        market: String::new(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_LICENSE_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_LICENSE_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: LicenseResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok((response.is_active, response.expiration_date))
}

/// `XStoreQueryEntitledProductsAsync`'s real backing - titles this account owns outright or
/// through a subscription (PC Game Pass / Game Pass Ultimate), from `xodus-service`'s "My
/// games" library lookup. Unlike [`get_game_license`], not gated on [`ENV_CONTENT_ID`]: this
/// always answers for whichever account is signed in, package or no package.
pub(crate) fn get_entitled_products(market: &str) -> Result<Vec<EntitledProduct>, HRESULT> {
    let request_body = EntitledProductsRequest {
        market: market.to_string(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_ENTITLED_PRODUCTS_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_ENTITLED_PRODUCTS_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: EntitledProductsResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok(response.products)
}

/// `XStoreGetUserCollectionsIdAsync`'s real backing, via `xodus-service`'s
/// `CollectionsIdRequest` handler - a real call against
/// `collections.mp.microsoft.com/v7.0/beneficiaries/me/keys` (endpoint confirmed via
/// static analysis of the real `xgameruntime.dll`'s embedded service-configuration blob,
/// not guessed). `service_ticket`/`publisher_user_id` are forwarded verbatim; the result
/// is an opaque signed blob meant for the title's own backend, returned as-is.
pub(crate) fn get_user_collections_id(
    service_ticket: &str,
    publisher_user_id: &str,
) -> Result<String, HRESULT> {
    let request_body = CollectionsIdRequest {
        service_ticket: service_ticket.to_string(),
        publisher_user_id: publisher_user_id.to_string(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_COLLECTIONS_ID_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_COLLECTIONS_ID_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: CollectionsIdResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok(response.key)
}

/// `XStoreQueryLicenseTokenAsync`'s real backing, via `xodus-service`'s
/// `LicenseTokenRequest` handler - a real call against
/// `licensing.mp.microsoft.com/v8.0/licenseToken` (endpoint confirmed the same way as
/// [`get_user_collections_id`]). The result is an opaque token meant for the title's own
/// backend, returned as-is.
pub(crate) fn get_license_token(
    product_ids: &[String],
    custom_developer_string: &str,
) -> Result<String, HRESULT> {
    let request_body = LicenseTokenRequest {
        product_ids: product_ids.to_vec(),
        custom_developer_string: custom_developer_string.to_string(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_LICENSE_TOKEN_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_LICENSE_TOKEN_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: LicenseTokenResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok(response.token)
}

/// `XStoreQueryAssociatedProductsAsync`'s real backing, via `xodus-service`'s
/// `AssociatedProductsRequest` handler - products "sellable by" (DLC/add-ons for) the
/// running game's own catalog entry. `Err(E_NOTIMPL)` when [`ENV_PACKAGE_FAMILY_NAME`] is
/// unset (not running under `xodus-cli run`, or `xodus-cli run` couldn't find/parse an
/// `AppxManifest.xml`) - same "nothing to resolve" stance as [`get_game_license`]'s gate on
/// [`ENV_CONTENT_ID`].
pub(crate) fn get_associated_products(
    max_items: u32,
) -> Result<Vec<AssociatedProductEntry>, HRESULT> {
    let package_family_name = std::env::var(ENV_PACKAGE_FAMILY_NAME).map_err(|_| E_NOTIMPL)?;
    let request_body = AssociatedProductsRequest {
        package_family_name,
        market: String::new(),
        max_items,
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_ASSOCIATED_PRODUCTS_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_ASSOCIATED_PRODUCTS_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: AssociatedProductsResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    Ok(response.products)
}

/// `XPersistentLocalStorageMountForPackage`'s real backing: resolves a `PackageFamilyName`
/// (the real interface's `packageIdentifier` - confirmed via strings recovered from the real
/// `xgameruntime.dll`, which implements `XPackageGetCurrentProcessPackageIdentifier` on top of
/// Win32's own `GetCurrentPackageFamilyName`) to a `StoreId`, via the same
/// `alternateid=PackageFamilyName` catalog lookup [`get_associated_products`] uses. `Ok(None)`
/// is a "no such product", not an error - the caller decides what that means.
pub(crate) fn resolve_product_id(package_family_name: &str) -> Result<Option<String>, HRESULT> {
    let request_body = ResolveProductIdRequest {
        package_family_name: package_family_name.to_string(),
        market: String::new(),
    };
    let body = quick_xml::se::to_string(&request_body).map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_RESOLVE_PRODUCT_ID_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_RESOLVE_PRODUCT_ID_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: ResolveProductIdResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    if response.product_id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(response.product_id))
    }
}

/// `XUserGetGamerPictureAsync`'s real backing. No user field - like [`get_entitled_products`],
/// `xodus-service` always answers for whichever account's credentials are on this connection.
/// `Ok(None)` when the account has no gamer picture set - an absence, not an error.
pub(crate) fn get_gamer_picture() -> Result<Option<Vec<u8>>, HRESULT> {
    let body = quick_xml::se::to_string(&GamerPictureRequest {
        client_id: title_client_id(),
    })
    .map_err(|_| E_FAIL)?;

    let (reply_type, reply_body) = request(MSG_TYPE_GAMER_PICTURE_REQUEST, body.as_bytes())?;
    if reply_type != MSG_TYPE_GAMER_PICTURE_RESPONSE {
        return Err(E_FAIL);
    }

    let text = std::str::from_utf8(&reply_body).map_err(|_| E_FAIL)?;
    let response: GamerPictureResponse = quick_xml::de::from_str(text).map_err(|_| E_FAIL)?;
    if response.picture.is_empty() {
        return Ok(None);
    }
    base64_decode(&response.picture).map(Some).ok_or(E_FAIL)
}

/// The launched package's declared `PersistentLocalStorage` size, in bytes, from
/// [`ENV_PLS_SIZE_MB`]/[`ENV_PLS_GROWABLE_TO_MB`]. `None` when unset - no `PersistentLocalStorage`
/// element was found in `MicrosoftGame.config`, and the caller should fall back to a placeholder.
pub(crate) fn persistent_local_storage_space() -> Option<(u64, u64)> {
    let shareable = std::env::var(ENV_PLS_SHAREABLE).ok()?;
    let _ = shareable; // presence, not value, is what marks the element as found
    let size_mb: u64 = std::env::var(ENV_PLS_SIZE_MB).ok()?.parse().ok()?;
    let growable_to_mb: u64 = std::env::var(ENV_PLS_GROWABLE_TO_MB)
        .ok()?
        .parse()
        .unwrap_or(size_mb);
    Some((
        size_mb * 1024 * 1024,
        growable_to_mb.max(size_mb) * 1024 * 1024,
    ))
}

/// Whether `store_id` is one of the launched package's declared `RelatedProducts`
/// (`XPersistentLocalStorageMountForPackage`'s eligibility check).
pub(crate) fn is_related_product(store_id: &str) -> bool {
    std::env::var(ENV_RELATED_PRODUCTS)
        .map(|list| list.split(',').any(|id| id == store_id))
        .unwrap_or(false)
}

/// The real, persistent game-save root `xodus-cli run` published, if any.
pub(crate) fn game_save_root() -> Option<String> {
    std::env::var(ENV_GAME_SAVE_ROOT).ok()
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
            std::env::remove_var(ENV_CONTENT_ID);
        }
    }

    fn set_content_id_env(content_id: &str) {
        unsafe {
            std::env::set_var(ENV_CONTENT_ID, content_id);
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
            let request: MSATokenRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.client_id, XBOX_LIVE_CLIENT_ID);
            assert!(!request.msa_full_trust);

            let response = MSATokenResponse {
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

    /// Regression test for the "Sign in with Microsoft" bug: a transient failure on the
    /// first token-and-signature fetch (surfaced here as `MSG_TYPE_ERROR`, mirroring
    /// `xodus_service::connection::xml::ERROR_REPLY_TYPE`) must not be fatal - a retry
    /// against a fresh connection that succeeds should be enough to get a real token.
    #[test]
    fn xsts_token_request_retries_past_a_transient_server_error() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x99u8; SECRET_LEN];

        let server = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().expect("accept");

                let mut magic = [0u8; 4];
                socket.read_exact(&mut magic).expect("read handshake magic");
                assert_eq!(u32::from_le_bytes(magic), HANDSHAKE_MAGIC);
                let presented = read_exact_blocking(&mut socket, SECRET_LEN);
                assert_eq!(presented, secret);
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
                let size =
                    u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
                let _body = read_exact_blocking(&mut socket, size);

                let (reply_type, payload) = if attempt == 0 {
                    (MSG_TYPE_ERROR, b"transient token exchange failure".to_vec())
                } else {
                    let response = XstsTokenResponse {
                        token: "fake-xsts-token".to_string(),
                        authorization: "XBL3.0 x=fake-uhs;fake-xsts-token".to_string(),
                        signature: "fake-signature".to_string(),
                        expiry: 1_700_000_000,
                    };
                    (
                        MSG_TYPE_XSTS_TOKEN_RESPONSE,
                        quick_xml::se::to_string(&response).unwrap().into_bytes(),
                    )
                };
                let mut reply = Vec::new();
                reply.extend(XML_MAGIC_V2.to_le_bytes());
                reply.extend(reply_type.to_le_bytes());
                reply.extend((payload.len() as u32).to_le_bytes());
                reply.extend(payload);
                socket.write_all(&reply).expect("write reply");
            }
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result =
            get_token_and_signature("GET", "https://profile.xboxlive.com/users/me", b"payload");
        clear_endpoint_env();
        server.join().expect("server thread");

        let (authorization, signature) = result.expect("retry recovers from the transient error");
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

    #[test]
    fn interactive_sign_in_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0xaau8; SECRET_LEN];
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
            assert_eq!(
                u16::from_le_bytes([header[0], header[1]]),
                MSG_TYPE_INTERACTIVE_SIGN_IN_REQUEST
            );
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let _body = read_exact_blocking(&mut socket, size);

            let response = InteractiveSignInResponse {
                success: true,
                xuid: "2533274999999999".to_string(),
                gamertag: "FakeGamer".to_string(),
                gamertag_modern: "FakeGamer".to_string(),
                age_group: "Adult".to_string(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_INTERACTIVE_SIGN_IN_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = interactive_sign_in();
        clear_endpoint_env();
        server.join().expect("server thread");

        let (xuid, gamertag, gamertag_modern, age_group) =
            result.expect("round trip succeeds").expect("signed in");
        assert_eq!(xuid, "2533274999999999");
        assert_eq!(gamertag, "FakeGamer");
        assert_eq!(gamertag_modern, "FakeGamer");
        assert_eq!(age_group, "Adult");
    }

    #[test]
    fn interactive_sign_in_reports_none_when_declined() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0xbbu8; SECRET_LEN];
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
            assert_eq!(
                u16::from_le_bytes([header[0], header[1]]),
                MSG_TYPE_INTERACTIVE_SIGN_IN_REQUEST
            );
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let _body = read_exact_blocking(&mut socket, size);

            let response = InteractiveSignInResponse {
                success: false,
                xuid: String::new(),
                gamertag: String::new(),
                gamertag_modern: String::new(),
                age_group: String::new(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_INTERACTIVE_SIGN_IN_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = interactive_sign_in();
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(result.expect("round trip succeeds"), None);
    }

    #[test]
    fn missing_content_id_is_reported_as_not_implemented_not_a_crash() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        clear_endpoint_env();
        assert_eq!(get_game_license().unwrap_err(), E_NOTIMPL);
    }

    #[test]
    fn license_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x77u8; SECRET_LEN];
        let secret_for_server = secret;
        let content_id = "01234567-89ab-cdef-0123-456789abcdef";

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
            assert_eq!(msg_type, MSG_TYPE_LICENSE_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: LicenseRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.content_id, "01234567-89ab-cdef-0123-456789abcdef");

            let response = LicenseResponse {
                is_active: true,
                expiration_date: 1_800_000_000,
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_LICENSE_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        set_content_id_env(content_id);
        let result = get_game_license();
        clear_endpoint_env();
        server.join().expect("server thread");

        let (is_active, expiration_date) = result.expect("round trip succeeds");
        assert!(is_active);
        assert_eq!(expiration_date, 1_800_000_000);
    }

    #[test]
    fn entitled_products_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x55u8; SECRET_LEN];
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
            assert_eq!(msg_type, MSG_TYPE_ENTITLED_PRODUCTS_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: EntitledProductsRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.market, "US");

            let response = EntitledProductsResponse {
                products: vec![
                    EntitledProduct {
                        store_id: "9ABC123".to_string(),
                        title: "Some Game".to_string(),
                        product_kind: "Game".to_string(),
                        included_in_game_pass: true,
                    },
                    EntitledProduct {
                        store_id: "9DEF456".to_string(),
                        title: "Another Game".to_string(),
                        product_kind: "Game".to_string(),
                        included_in_game_pass: false,
                    },
                ],
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_ENTITLED_PRODUCTS_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_entitled_products("US");
        clear_endpoint_env();
        server.join().expect("server thread");

        let products = result.expect("round trip succeeds");
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].store_id, "9ABC123");
        assert!(products[0].included_in_game_pass);
        assert_eq!(products[1].store_id, "9DEF456");
        assert!(!products[1].included_in_game_pass);
    }

    #[test]
    fn empty_entitled_products_response_round_trips() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x66u8; SECRET_LEN];
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
            assert_eq!(
                u16::from_le_bytes([header[0], header[1]]),
                MSG_TYPE_ENTITLED_PRODUCTS_REQUEST
            );
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let _body = read_exact_blocking(&mut socket, size);

            let response = EntitledProductsResponse { products: vec![] };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_ENTITLED_PRODUCTS_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_entitled_products("US");
        clear_endpoint_env();
        server.join().expect("server thread");

        assert!(result.expect("round trip succeeds").is_empty());
    }

    #[test]
    fn collections_id_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x88u8; SECRET_LEN];
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
            assert_eq!(msg_type, MSG_TYPE_COLLECTIONS_ID_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: CollectionsIdRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.service_ticket, "fake-service-ticket");
            assert_eq!(request.publisher_user_id, "fake-publisher-user-id");

            let response = CollectionsIdResponse {
                key: "fake-collections-key".to_string(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_COLLECTIONS_ID_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_user_collections_id("fake-service-ticket", "fake-publisher-user-id");
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(result.expect("round trip succeeds"), "fake-collections-key");
    }

    #[test]
    fn license_token_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x33u8; SECRET_LEN];
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
            assert_eq!(msg_type, MSG_TYPE_LICENSE_TOKEN_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: LicenseTokenRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(
                request.product_ids,
                vec!["9ABC123".to_string(), "9DEF456".to_string()]
            );
            assert_eq!(request.custom_developer_string, "custom-string");

            let response = LicenseTokenResponse {
                token: "fake-license-token".to_string(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_LICENSE_TOKEN_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_license_token(
            &["9ABC123".to_string(), "9DEF456".to_string()],
            "custom-string",
        );
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(result.expect("round trip succeeds"), "fake-license-token");
    }

    #[test]
    fn resolve_product_id_request_round_trips_against_a_fake_service() {
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
            assert_eq!(msg_type, MSG_TYPE_RESOLVE_PRODUCT_ID_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let body = read_exact_blocking(&mut socket, size);
            let request: ResolveProductIdRequest =
                quick_xml::de::from_str(std::str::from_utf8(&body).unwrap()).expect("parses");
            assert_eq!(request.package_family_name, "Example.Game_8wekyb3d8bbwe");

            let response = ResolveProductIdResponse {
                product_id: "9NABC1234567".to_string(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_RESOLVE_PRODUCT_ID_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = resolve_product_id("Example.Game_8wekyb3d8bbwe");
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(
            result.expect("round trip succeeds"),
            Some("9NABC1234567".to_string())
        );
    }

    #[test]
    fn gamer_picture_request_round_trips_against_a_fake_service() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x55u8; SECRET_LEN];
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
            assert_eq!(msg_type, MSG_TYPE_GAMER_PICTURE_REQUEST);
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let _body = read_exact_blocking(&mut socket, size);

            let response = GamerPictureResponse {
                picture: base64_encode(b"not-really-a-png"),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_GAMER_PICTURE_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_gamer_picture();
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(
            result.expect("round trip succeeds"),
            Some(b"not-really-a-png".to_vec())
        );
    }

    #[test]
    fn gamer_picture_request_reports_none_when_account_has_no_picture() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = [0x33u8; SECRET_LEN];
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
            assert_eq!(
                u16::from_le_bytes([header[0], header[1]]),
                MSG_TYPE_GAMER_PICTURE_REQUEST
            );
            let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
            let _body = read_exact_blocking(&mut socket, size);

            let response = GamerPictureResponse {
                picture: String::new(),
            };
            let payload = quick_xml::se::to_string(&response).unwrap().into_bytes();
            let mut reply = Vec::new();
            reply.extend(XML_MAGIC_V2.to_le_bytes());
            reply.extend(MSG_TYPE_GAMER_PICTURE_RESPONSE.to_le_bytes());
            reply.extend((payload.len() as u32).to_le_bytes());
            reply.extend(payload);
            socket.write_all(&reply).expect("write reply");
        });

        set_endpoint_env(port, &hex_encode(&secret));
        let result = get_gamer_picture();
        clear_endpoint_env();
        server.join().expect("server thread");

        assert_eq!(result.expect("round trip succeeds"), None);
    }

    #[test]
    fn base64_decode_matches_known_vectors() {
        assert_eq!(base64_decode(""), Some(Vec::new()));
        assert_eq!(base64_decode("Zg=="), Some(b"f".to_vec()));
        assert_eq!(base64_decode("Zm8="), Some(b"fo".to_vec()));
        assert_eq!(base64_decode("Zm9v"), Some(b"foo".to_vec()));
        assert_eq!(base64_decode("Zm9vYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(base64_decode("not valid base64!"), None);
    }

    #[test]
    fn persistent_local_storage_space_reads_env_vars_and_falls_back_when_unset() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var(ENV_PLS_SIZE_MB);
            std::env::remove_var(ENV_PLS_GROWABLE_TO_MB);
            std::env::remove_var(ENV_PLS_SHAREABLE);
        }
        assert_eq!(persistent_local_storage_space(), None);

        unsafe {
            std::env::set_var(ENV_PLS_SIZE_MB, "128");
            std::env::set_var(ENV_PLS_GROWABLE_TO_MB, "512");
            std::env::set_var(ENV_PLS_SHAREABLE, "true");
        }
        assert_eq!(
            persistent_local_storage_space(),
            Some((128 * 1024 * 1024, 512 * 1024 * 1024))
        );
        unsafe {
            std::env::remove_var(ENV_PLS_SIZE_MB);
            std::env::remove_var(ENV_PLS_GROWABLE_TO_MB);
            std::env::remove_var(ENV_PLS_SHAREABLE);
        }
    }

    #[test]
    fn is_related_product_checks_the_declared_list() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var(ENV_RELATED_PRODUCTS, "9NABC1234567,9NDEF7654321");
        }
        assert!(is_related_product("9NABC1234567"));
        assert!(!is_related_product("9NOTLISTED0000"));
        unsafe {
            std::env::remove_var(ENV_RELATED_PRODUCTS);
        }
        assert!(!is_related_product("9NABC1234567"));
    }
}
