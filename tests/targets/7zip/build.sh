#!/usr/bin/env bash
# Build 7zz.exe (7-Zip's standalone Alone2 console, 64-bit) with MinGW-w64
# cross-compiling from Linux.
#   tests/targets/7zip/build.sh SOURCE.tar.xz [OUTDIR]
# SOURCE is the mirrored src-7zip-<version> asset (sha256 checked against
# tests/targets/sources.json); OUTDIR defaults to target/targets. Writes
# OUTDIR/7zz.exe and records the make command line in OUTDIR/7zz.make-args.
#
# 7-Zip's own makefiles do the work (DOC/readme.txt: `make -f ../../cmpl_gcc.mak`
# in CPP/7zip/Bundles/Alone2). Nothing in the 7-Zip source is patched. The
# build-system tweaks, all variables on the make command line:
#   CROSS_COMPILE=x86_64-w64-mingw32-   use the MinGW compilers (cmpl_gcc.mak
#                                       reads $(CROSS_COMPILE)gcc / g++).
#   IS_MINGW=1                          7zip_gcc.mak detects MinGW through the
#                                       Windows environment variable SystemDrive,
#                                       which a Linux host doesn't have. This
#                                       selects the Windows sources, -static and
#                                       the .exe suffix.
#   RC=x86_64-w64-mingw32-windres       its default is windres.exe.
#   CFLAGS_BASE2 / CXXFLAGS_BASE2       add one -I directory of generated shim
#                                       headers. 7-Zip includes Windows headers
#                                       with Windows capitalization (<NTSecAPI.h>,
#                                       <Windows.h>); MinGW-w64's are lowercase and
#                                       a Linux file system is case sensitive.
#                                       Each shim is `#include <lowercase.h>`.
#   MY_MKDIR="mkdir -p" RM="rm -f"      the makefile picks `mkdir`/`del` (Windows
#                                       cmd) unless MSYSTEM is set; `mkdir -p`
#                                       also works with parallel make.
#   LFLAGS_STRIP=                       upstream links with -s (strip); we keep
#                                       the COFF symbols for the census and for
#                                       readable debugging.
#   LDFLAGS_STATIC_3="-static-libgcc -static-libstdc++ -L<shim>"
#                                       with IS_MINGW the makefile already passes
#                                       -static, which links libgcc, libstdc++ and
#                                       winpthread statically; this repeats it so
#                                       the intent is explicit. The extra -L
#                                       directory holds generated symlinks: the
#                                       link line names import libraries with
#                                       Windows capitalization (-lShell32,
#                                       -lUser32) and MinGW-w64's are lowercase.
# Not set, on purpose: USE_ASM (the C code paths are what we translate), IS_X64
# (it only selects assembler flags), ST_MODE (7-Zip's own threads stay on; run
# with -mmt1 for a single compression thread). Compiler flags stay upstream's
# (-O2 -DNDEBUG -D_REENTRANT -D_FILE_OFFSET_BITS=64 -fPIC, -Werror -Wall -Wextra).
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: $0 SOURCE.tar.xz [OUTDIR]" >&2; exit 2; }
tarball=$(realpath "$1")
out=${2:-$root/target/targets}
mkdir -p "$out"
out=$(realpath "$out")

want=$(jq -r '."7zip".sha256' "$root/tests/targets/sources.json")
have=$(sha256sum "$tarball" | awk '{print $1}')
if [ "$want" != "$have" ]; then
  echo "7zip/build.sh: sha256 of $tarball is $have, sources.json pins $want" >&2
  exit 1
fi

work="$root/target/targets/7zip-build"
rm -rf "$work"
mkdir -p "$work"
tar -C "$work" -xf "$tarball"

# Headers 7-Zip 25.01 includes with a capitalization MinGW-w64 doesn't have.
shim="$work/shim-include"
mkdir -p "$shim"
for h in CommCtrl HtmlHelp MAPI NTSecAPI OleCtl Psapi ShObjIdl ShlGuid ShlObj Shlwapi \
  TlHelp32 WinUser WinVer Windows; do
  lower=$(echo "$h" | tr 'A-Z' 'a-z')
  printf '#include <%s.h>\n' "$lower" > "$shim/$h.h"
done

# Import libraries the makefile names with a capital letter.
shimlib="$work/shim-lib"
mkdir -p "$shimlib"
for l in Ole32 Gdi32 Comctl32 Comdlg32 Shell32 User32; do
  lower=$(echo "$l" | tr 'A-Z' 'a-z')
  target=$(x86_64-w64-mingw32-gcc -print-file-name="lib$lower.a")
  [ -f "$target" ] || { echo "7zip/build.sh: lib$lower.a not found" >&2; exit 1; }
  ln -s "$target" "$shimlib/lib$l.a"
done

args=(
  "CFLAGS_BASE2=-I$shim"
  "CXXFLAGS_BASE2=-I$shim"
  CROSS_COMPILE=x86_64-w64-mingw32-
  IS_MINGW=1
  RC=x86_64-w64-mingw32-windres
  "MY_MKDIR=mkdir -p"
  "RM=rm -f"
  LFLAGS_STRIP=
  "LDFLAGS_STATIC_3=-static-libgcc -static-libstdc++ -L$shimlib"
)
cd "$work/CPP/7zip/Bundles/Alone2"
make -j"${JOBS:-$(nproc)}" -f ../../cmpl_gcc.mak "${args[@]}"

exe=$(ls b/g/7zz.exe)
cp "$exe" "$out/7zz.exe"
printf '%s\n' "${args[@]}" > "$out/7zz.make-args"
echo "built $out/7zz.exe"
sha256sum "$out/7zz.exe"
