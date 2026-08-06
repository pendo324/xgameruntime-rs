# xgameruntime-unixlib

The native-Linux half of `xgameruntime.dll`'s IPC. It exists for one reason: Wine's Winsock has no `AF_UNIX` — `dlls/ntdll/unix/socket.c` converts only INET/INET6/IPX/IRDA/UNSPEC — so a PE cannot open `xodus-service`'s `xodus.sock` by any Winsock route. This library can, because it *is* Linux code, and the DLL calls into it through Wine's `__wine_unix_call`.

The DLL prefers this over its loopback TCP transport because a Unix socket authenticates its peer (mode bits, `SO_PEERCRED`), while the TCP port can only check a shared secret that any same-uid process can read out of the game's environment. It is not a latency optimization — round trips are dominated by whatever `xodus-service` does upstream. When this library is absent, the DLL falls back to TCP on its own; nothing here is required for a working launch.

## How it gets loaded

Wine `dlopen`s a `.so` beside a PE only for modules it resolved as **builtins**, so `xgameruntime.dll` is shipped as one. Three things have to line up, and `xodus-cli run-umu` arranges all three:

1. The DLL carries the 32-byte `"Wine builtin DLL"` signature at file offset `0x40`, stamped by `winebuild --builtin` in `scripts/build-release.sh`. Without it the loader classifies the file as native and never looks for a `.so` at all.
2. A copy sits in the prefix's `system32`, because `LdrLoadDll` has to find *a* file by that name on the normal search path before any of the builtin machinery engages — the same reason Wine puts fake-DLL placeholders in every prefix.
3. The PE and this `.so` are linked into the Proton runtime's own `files/lib/wine/{x86_64-windows,x86_64-unix}/`. `find_builtin_dll` (`dlls/ntdll/unix/loader.c`) derives the `.so` name by swapping the PE's extension, so the filenames must match.

The loadorder is `xgameruntime=b,n`, *not* `=n`: a native override makes `load_builtin` bail before it reaches the pairing.

Note: the libraries (.dll/.so combination) are installed in the runtime's directory, _not_ the prefix.

`set_dll_path` seeds `dll_paths[]` with the runtime's own `lib/wine` *unconditionally, before* any `WINEDLLPATH` entry, and never consults the prefix or `system32` at all. The Proton fork these titles launch under ships its own `xgameruntime.dll` there, so a copy staged elsewhere and pointed at with `WINEDLLPATH` loses to it silently and the build you asked to run never loads.

That directory is shared with every other game launched under the same Proton, so `xodus-cli run-umu` installs symlinks and moves the runtime's originals aside to `*.xodus-orig` rather than overwriting them — restoring is a matter of moving them back, a rebuild of the DLL takes effect without reinstalling, and re-running is idempotent because it only ever displaces a real file, never a symlink it previously made.

The PE side also accepts a handle published through `XODUS_UNIXLIB_HANDLE` by this library's ELF constructor, which is how an `LD_PRELOAD`ed copy can be reached without any of the above. That path works — Wine's dispatcher does a raw `callq *(%r10,%rdx,8)` and never validates the handle's provenance — and is kept as a fallback, but it is no longer how deployment works.

## Building

Use `scripts/build-release.sh` from the repository root; it builds both halves and applies the signature. To build this half alone:

```sh
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17
```

`cargo zigbuild` pins a glibc floor old enough for the runtimes games actually launch under; the host toolchain would otherwise bake in whatever the build machine happens to have. The output is `target/x86_64-unknown-linux-gnu/release/libxgameruntime.so`, which deploys as `xgameruntime.so` beside `xgameruntime.dll`.

Plain `cargo build`/`cargo test` here target the host, via `.cargo/config.toml` — the parent crate's config forces `x86_64-pc-windows-msvc`, which is right for the DLL and wrong here.

## ABI

The parameter structs are hand-mirrored in `../src/ipc/unixlib.rs`. Nothing at build time links the two halves, so both sides assert the same field offsets in their own test suites — a field reordered on one side would otherwise be silently misread on the other, at runtime, with a game's credentials in the buffer.

The call codes (`CALL_EXCHANGE`, `CALL_FETCH_REPLY`) are indices into `__wine_unix_call_funcs` and may only be appended to.

## Testing the round trip for real

The end-to-end test is `#[ignore]`d in the parent crate because it needs a listener and the preload. To run it against a stub service:

```sh
# Terminal 1 — a stub that answers any request with <Resp>hello</Resp>
python3 - /tmp/xodus-test.sock <<'PY'
import os, socket, struct, sys
path = sys.argv[1]
if os.path.exists(path): os.unlink(path)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.bind(path); s.listen(4)
while True:
    c, _ = s.accept()
    magic, mtype, size = struct.unpack("<IHI", c.recv(10))
    body = b""
    while len(body) < size: body += c.recv(size - len(body))
    reply = b"<Resp>hello</Resp>"
    c.sendall(struct.pack("<IHI", 0x59445358, mtype + 1, len(reply)) + reply)
    c.close()
PY

# Terminal 2
cd .. && XODUS_DIAG=1 \
  XODUS_SOCKET_PATH=/tmp/xodus-test.sock \
  LD_PRELOAD=$PWD/unixlib/target/x86_64-unknown-linux-gnu/release/libxgameruntime.so \
  cargo test --target x86_64-pc-windows-msvc round_trip_over_the_unix_socket -- --ignored
```
