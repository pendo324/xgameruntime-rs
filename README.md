# xgameruntime rust

This is not yet a full rust implementation.

`xgameruntime.gdk.dll` is the official gdk sdk `.dll` renamed and needed for this wrapper to work.

## Building

The crate only targets `x86_64-pc-windows-msvc` — it is a DLL loaded by GDK games, and
the `windows-*` crates it is built on support no other target. There is no host-native
build, so on Linux you cross-compile, using clang plus Wine's own import libraries in
place of MSVC's. `.cargo/config.toml` wires this up; nothing needs to be passed on the
command line.

```bash
cargo check
cargo build              # produces target/x86_64-pc-windows-msvc/debug/xgameruntime.dll
cargo test               # test binaries are PE executables, run through Wine
```

Prerequisites on Linux: `rustup target add x86_64-pc-windows-msvc`, `clang`, `lld`, and
Wine including `winegcc`/`wineg++` and its `x86_64-windows` import libraries. Those
libraries are found automatically under the usual prefixes; for a self-built Wine, set
`WINE_MSVC_LIB_DIR` in a `.env` file at the repo root.

On Windows the same `cargo build` works once the MSVC toolchain overrides in
`.github/workflows/test.yml` are in the environment.

Cross-compiling this way is adapted from
[PR #1](https://github.com/minecraft-linux/xgameruntime-rs/pull/1).
