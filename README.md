# xgameruntime rust

A Rust reimplementation of the GDK's `xgameruntime.dll`, built to be loaded by Xbox PC (GDK) titles running under Wine/Proton. It is not yet a full implementation — it covers the surface those titles touch during startup and sign-in, and defers the rest.

Most of what the DLL cannot answer on its own it forwards to [`xodus-service`](https://github.com/xodus-gaming/xodus), which holds the real Xbox Live credentials. Two transports exist for that: a loopback TCP socket the PE can open through Winsock, and a Unix socket reached through a native `.so` half — see [`unixlib/README.md`](unixlib/README.md) for why both exist and which one wins.

> [!CAUTION]
> Unofficial project. Not affiliated with, endorsed by, or sponsored by Microsoft or Xbox.

> [!NOTE]
> **AI disclaimer.** Substantial portions of this repository were written with AI assistance. Everything here has been exercised against a real instance of Minecraft Bedrock running under Wine/Proton. Review the code on its own merits rather than assuming a human wrote every line.

## Building

Use the script:

```bash
./scripts/build-release.sh
```

It produces the two files that deploy together:

```
target/x86_64-pc-windows-msvc/release/xgameruntime.dll
target/x86_64-pc-windows-msvc/release/xgameruntime.so
```

A plain `cargo build` is **not** a substitute. It builds only the PE half, and it skips the post-link step cargo has no hook for: stamping the 32-byte `"Wine builtin DLL"` signature at file offset `0x40` with `winebuild --builtin`. That signature decides the whole loading strategy, and the two strategies are mutually exclusive — signed, the DLL loads as a builtin and Wine will pair the `.so` with it, but `xgameruntime=n` makes `LoadLibrary` fail outright; unsigned, it loads natively and the `.so` is never looked for. The script's header comment carries the details.

The DLL half targets `x86_64-pc-windows-msvc` and the `windows-*` crates it is built on support no other target, so on Linux you cross-compile using clang plus Wine's own import libraries in place of MSVC's. `.cargo/config.toml` wires this up; nothing needs to be passed on the command line.

```bash
cargo check
cargo test               # test binaries are PE executables, run through Wine
```

Prerequisites on Linux: `rustup target add x86_64-pc-windows-msvc`, `clang`, `lld`, `cargo-zigbuild` (for the native half), and Wine including `winebuild`, `winegcc`/`wineg++`, and its `x86_64-windows` import libraries. Those libraries are found automatically under the usual prefixes; for a self-built Wine, set `WINE_MSVC_LIB_DIR` in a `.env` file at the repo root.

On Windows `cargo build` works once the MSVC toolchain overrides in `.github/workflows/test.yml` are in the environment. The `.so` half is Linux-only and has no Windows equivalent — there is nothing for it to do there.

Cross-compiling this way is adapted from [PR #1](https://github.com/minecraft-linux/xgameruntime-rs/pull/1).

## Running a title against it

Deployment is not a matter of dropping the DLL next to the game. Wine's builtin search (`find_builtin_dll`, `dlls/ntdll/unix/loader.c`) scans the runtime's own `lib/wine` before any `WINEDLLPATH` entry, so a Proton build shipping its own `xgameruntime.dll` silently wins over anything staged elsewhere. `xodus-cli run-umu` handles the placement, the `WINEDLLOVERRIDES` loadorder, and preserving the runtime's originals:

```bash
xodus-cli run-umu <game-dir-or-product-id> \
  /path/to/xgameruntime-rs/target/x86_64-pc-windows-msvc/release/xgameruntime.dll \
  --proton /path/to/proton-xodus
```

## Layout

```
src/       the PE half — COM interfaces, XGameRuntime exports, IPC client
unixlib/   the native half, loaded by Wine through __wine_unix_call (see its README)
scripts/   build-release.sh, the supported build entry point
```
