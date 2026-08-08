#!/bin/sh
# Builds both halves of the runtime: the PE and the native library it calls into.
#
# The two halves have to be built separately because they target different triples. Hence a
# script rather than a build.rs.
#
# The PE also gets a post-link step cargo has no hook for: the "Wine builtin DLL" signature,
# which is what makes Wine pair the `.so` with it. `find_builtin_dll`
# (dlls/ntdll/unix/loader.c) derives the `.so` name by swapping the PE's extension, but only
# for modules it resolves as builtins.
#
# The signature is not optional-but-nice - it decides the whole loading strategy, and the two
# strategies are mutually exclusive:
#
#   * Signed, the DLL *cannot* be loaded with `xgameruntime=n`. `load_builtin` opens with
#     `if (image_info->wine_builtin) { if (loadorder == LO_NATIVE) return STATUS_DLL_NOT_FOUND; }`
#     so LoadLibrary fails outright and the title exits during startup.
#   * Unsigned, it loads natively but Wine never looks for the `.so`, so the unix half is
#     unreachable and the DLL falls back to loopback TCP.
#
# Signed is the one we want, which puts the burden on the *deployer* to make sure the builtin
# search reaches this file: `find_builtin_dll` scans the runtime's own `lib/wine` before any
# `WINEDLLPATH` entry, so a runtime shipping its own `xgameruntime.dll` wins unless the pair
# is installed into that directory. `xodus-cli run-umu` does exactly that.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# `.2.17` pins a glibc floor old enough for the runtimes games actually launch under; a
# plain cargo build would bind to the build host's glibc. Both host arches are built from
# whatever machine runs this script - zig's bundled cross-linker/sysroot supplies the C
# side of cross-linking, but rustc's own std/core for the non-host triple still has to
# come from rustup, hence the `target add` for both (a no-op for whichever one is
# already the host's default).
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
echo ">>> unixlib (native, via zig)"
(cd unixlib && cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17)
(cd unixlib && cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.17)

echo ">>> dll (x86_64-pc-windows-msvc)"
cargo build --release

dll="target/x86_64-pc-windows-msvc/release/xgameruntime.dll"
so_x86_64="unixlib/target/x86_64-unknown-linux-gnu/release/libxgameruntime.so"
so_aarch64="unixlib/target/aarch64-unknown-linux-gnu/release/libxgameruntime.so"

# `winebuild --builtin` writes the 32-byte signature over the DOS stub at file offset 0x40,
# where the server looks for it (server/mapping.c). Using Wine's own tool rather than patching
# by hand is not just tidiness: it refuses the file unless `e_lfanew >= 96`, so a future linker
# change that shrank the DOS stub fails here instead of silently clobbering the PE headers.
# Re-running it is harmless, but cargo will not relink an unchanged crate, so the stamp has to
# survive a no-op build - hence checking rather than stamping unconditionally.
# `-a` is load-bearing: the 32-byte field is NUL-padded, so grep classifies the input as
# binary and reports no match even when the signature is there. Without it this test always
# failed and the stamp was reapplied on every build - harmless, but it defeated the point of
# checking, and the same idiom silently misreports elsewhere.
if head -c 96 "$dll" | tail -c +65 | LC_ALL=C grep -qa "Wine builtin DLL"; then
    echo ">>> $dll is already marked as a Wine builtin"
else
    echo ">>> marking $dll as a Wine builtin"
    winebuild --builtin "$dll"
fi

# Wine's find_builtin_dll pairs a `.so` with its PE by swapping the extension, so the file
# that actually loads at deploy time must be plain `xgameruntime.so` - these arch-suffixed
# names exist so a single build output can carry both variants for a cross-arch deployer
# (xodus's combine-proton.sh) to pick from and rename. A same-host deployer needs no
# renaming step of its own, so the native arch's copy is also dropped in under the plain
# name - this is what `xodus-cli run-umu` (via hack/build.sh) installs directly.
cp "$so_x86_64" "target/x86_64-pc-windows-msvc/release/xgameruntime-x86_64.so"
cp "$so_aarch64" "target/x86_64-pc-windows-msvc/release/xgameruntime-aarch64.so"
case $(uname -m) in
    x86_64) native_so=$so_x86_64 ;;
    aarch64|arm64) native_so=$so_aarch64 ;;
    *) echo "!! unsupported host arch: $(uname -m)" >&2; exit 1 ;;
esac
cp "$native_so" "target/x86_64-pc-windows-msvc/release/xgameruntime.so"

# A sibling .sha512sum per file a deployer downloads independently (xodus's
# combine-proton.sh fetches the DLL and each arch's .so as separate CI artifacts) lets it
# verify what it pulled without needing to trust the transfer alone.
(
    cd target/x86_64-pc-windows-msvc/release
    for f in xgameruntime.dll xgameruntime-x86_64.so xgameruntime-aarch64.so; do
        sha512sum "$f" > "$f.sha512sum"
    done
)

echo
echo "built:"
echo "  $root/$dll"
echo "  $root/target/x86_64-pc-windows-msvc/release/xgameruntime.so"
echo "  $root/target/x86_64-pc-windows-msvc/release/xgameruntime-x86_64.so"
echo "  $root/target/x86_64-pc-windows-msvc/release/xgameruntime-aarch64.so"
echo "  (plus a .sha512sum beside the dll and each arch-suffixed .so)"
