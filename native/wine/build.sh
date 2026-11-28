#!/usr/bin/env bash
# Build Wine 11's ntdll Unix part, wineserver and win32u's Unix part with
# Emscripten as one wasm64 module (docs/design/wine-unix.md, section 2).
#
#   native/wine/build.sh [--out DIR] [--jobs N] [--census]
#
# DIR (default target/wine-unix, inside the repository: Docker mounts the
# repository at /src) receives everything:
#   src/wine-11.0.tar.xz      the pinned tarball (tests/targets/sources.json, "wine")
#   src/wine-11.0/            what is compiled, patched with native/wine/patches/
#   gen/syscall_table.c       the syscall wrappers (native/wine/gen-syscalls.mjs)
#   obj/                      objects, build.mk, logs
#   wine-unix.{mjs,wasm}      the module
#   test-NAME.{mjs,wasm}      one per native/wine/tests/NAME.c
#   build-info.json           emcc version, tarball and patch hashes, imports, sizes
# Then the four gates: no undefined symbols (the link), every import listed in
# native/wine/imports.txt, no global symbol defined by two of the three parts
# (the server, ntdll with src/, win32u) that native/wine/rename-server.h or
# rename-win32u.h doesn't rename (--census prints the collisions), and the
# syscall generator's checks for tables 0 and 1.
# Needs Docker (scripts/emcc.sh), curl, patch, xz, gzip and Node 24.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$repo"
out=target/wine-unix
jobs="$(nproc 2>/dev/null || echo 4)"
census=0
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --jobs) jobs="$2"; shift 2 ;;
    --census) census=1; shift ;;
    -h|--help) sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
mkdir -p "$out"
out="$(cd "$out" && pwd -P)"
case "$out/" in
  "$repo"/*) ;;
  *) echo "build.sh: --out must be inside the repository ($repo): Docker mounts only it" >&2; exit 2 ;;
esac
rel="${out#"$repo"/}"
wn=native/wine
emcc_sh=scripts/emcc.sh
start=$(date +%s)
log() { echo "build.sh: $*"; }
die() { echo "build.sh: $*" >&2; exit 1; }

# ---- 1. Source --------------------------------------------------------------
read -r url sha256 version < <(node -e '
  const s = require(process.argv[1]).wine;
  console.log(s.url, s.sha256, s.version);' "$repo/tests/targets/sources.json")
tarball="$out/src/wine-$version.tar.xz"
mkdir -p "$out/src"
if [ ! -f "$tarball" ] || [ "$(sha256sum "$tarball" | cut -d' ' -f1)" != "$sha256" ]; then
  log "fetching $url"
  curl -fsSL --retry 3 -o "$tarball.part" "$url"
  mv "$tarball.part" "$tarball"
fi
got="$(sha256sum "$tarball" | cut -d' ' -f1)"
[ "$got" = "$sha256" ] || die "$tarball: sha256 $got, expected $sha256 (tests/targets/sources.json)"

# ---- 2. Extract and patch (again only when the tarball or a patch changed) ---
tree="$out/src/wine-$version"
treerel="$rel/src/wine-$version"
members=("wine-$version/dlls/ntdll/unix" "wine-$version/dlls/ntdll/*.h" "wine-$version/dlls/ntdll/Makefile.in"
         "wine-$version/server" "wine-$version/include" "wine-$version/COPYING.LIB"
         "wine-$version/dlls/win32u")
patches=()
while IFS= read -r p; do patches+=("$p"); done < <(find "$wn/patches" -name '*.patch' 2>/dev/null | LC_ALL=C sort)
stamp="$( { echo "$sha256"; printf '%s\n' "${members[@]}"; \
            for p in ${patches[@]+"${patches[@]}"}; do sha256sum "$p"; done; } | sha256sum | cut -d' ' -f1)"
if [ "$(cat "$tree/.tw-stamp" 2>/dev/null || true)" != "$stamp" ]; then
  log "extracting $(basename "$tarball") and applying ${#patches[@]} patches"
  rm -rf "$tree" "$out/obj"
  tar -C "$out/src" --wildcards -xJf "$tarball" "${members[@]}"
  for p in ${patches[@]+"${patches[@]}"}; do
    patch -d "$tree" -p1 --fuzz=0 --no-backup-if-mismatch -s -f < "$p" \
      || die "$p does not apply cleanly to the pristine tree (no fuzz)"
  done
  # configure would write include/config.h; ours is native/wine/config.h,
  # force-included, so `#include "config.h"` only needs to find a file.
  printf '/* native/wine/config.h is force-included (build.sh). */\n#include "/src/%s/config.h"\n' "$wn" \
    > "$tree/include/config.h"
  echo "$stamp" > "$tree/.tw-stamp"
fi

# The headers Wine's build generates with widl from include/*.idl, which
# win32u's sources include (wine-unix.md, section 2, step 1): from the native
# tree of tests/targets/wine.lock's release, built from the same tarball, as
# tests/wine/prepare.sh unpacks it. Each NAME.h there that the tarball's
# include/ lacks and whose include/NAME.idl it has (widl's output; not every
# .idl makes a header) goes to gen/include, after the tarball's include/.
wine_tag=$(cat tests/targets/wine.lock)
native_inc=target/wine/native/opt/wine-$version/include/wine/windows
[ "$(cat target/wine/.tag 2>/dev/null || true)" = "$wine_tag" ] && [ -d "$native_inc" ] \
  || die "the generated headers come from $wine_tag's native tree in target/wine/native: run tests/wine/prepare.sh (its step 1 unpacks it)"
mkdir -p "$out/gen/include.new"
for idl in "$tree"/include/*.idl; do
  h="$(basename "$idl" .idl).h"
  [ -f "$native_inc/$h" ] && [ ! -e "$tree/include/$h" ] || continue
  cp -p "$native_inc/$h" "$out/gen/include.new/$h"
done
# Replaced only when it changes, so make doesn't rebuild every object.
if diff -rq "$out/gen/include.new" "$out/gen/include" >/dev/null 2>&1; then rm -rf "$out/gen/include.new"
else rm -rf "$out/gen/include"; mv "$out/gen/include.new" "$out/gen/include"; fi
ngenh=$(ls "$out/gen/include" | wc -l)

# The syscall wrappers (wine-unix.md, section 11), from the patched tree at
# every build; written only when they change. Gate 4: the generator's checks.
node "$wn/gen-syscalls.mjs" "$tree" "$out/gen/syscall_table.c" \
  || die "gate 4 (syscalls) failed: native/wine/gen-syscalls.mjs could not match the entries above (wine-unix.md, section 11)"

# ---- 3./4. Compile and link (one make inside the Emscripten container) --------
# The source lists come from Wine's own makefiles.
mk_sources() { # FILE PREFIX: the .c entries of SOURCES that start with PREFIX
  awk -v pre="$2" '/^SOURCES *=/{f=1;next} f{ for(i=1;i<=NF;i++) if($i ~ /\.c$/ && index($i,pre)==1) print $i; if ($0 !~ /\\$/) f=0 }' "$1"
}
ntdll_srcs=$(mk_sources "$tree/dlls/ntdll/Makefile.in" unix/ | grep -v '^unix/signal_' | sed 's|^unix/||')
server_srcs=$(mk_sources "$tree/server/Makefile.in" "")
# win32u's Unix part: the SOURCES whose file has `#pragma makedep unix`
# (all but main.c, the PE side; dibdrv/ included).
win32u_srcs=$(for f in $(mk_sources "$tree/dlls/win32u/Makefile.in" ""); do
  if grep -q '^#pragma makedep unix' "$tree/dlls/win32u/$f"; then echo "$f"; fi; done)
own_srcs=$(cd "$wn/src" && ls *.c)
tests=$(cd "$wn/tests" && ls *.c 2>/dev/null | sed 's/\.c$//' || true)
[ -n "$ntdll_srcs" ] && [ -n "$server_srcs" ] && [ -n "$win32u_srcs" ] || die "no sources found in Wine's makefiles"

obj="$rel/obj"
mkdir -p "$out/obj/ntdll" "$out/obj/server" "$out/obj/win32u/dibdrv" "$out/obj/own" "$out/obj/gen" "$out/obj/tests"
# The host entries of abi.md v5 ("Host entries of the Unix side").
exports_common="_tw_syscall_dispatcher,_tw_unix_call_dispatcher,_malloc,_free,_tw_page_flags,_tw_main_state,_tw_next_timeout,_tw_run_timeouts,_emscripten_stack_set_limits,__emscripten_stack_restore,_tw_thread_main,_tw_exit_status"
mk="$out/obj/build.mk"
{
  echo "# Generated by native/wine/build.sh; runs in the Emscripten container at /src."
  echo "T := $treerel"
  echo "O := $obj"
  echo "W := $wn"
  echo "MK := $obj/build.mk"
  echo "CFLAGS := -m64 -O2 -sSHARED_MEMORY -fwasm-exceptions -sSUPPORT_LONGJMP=wasm -DWINE_UNIX_LIB -include \$(W)/config.h -MMD -MP"
  # The defines Wine's makefiles give each directory (makedep: -D__WINESRC__,
  # then EXTRADEFS; loader.c and unicode.c get the install paths, which are
  # wine-unix.md section 8's constant paths).
  echo "NTDLL := -D__WINESRC__ -D_NTSYSTEM_ -D_ACRTIMP= -DWINBASEAPI= -I\$(W)/include -I\$(T)/dlls/ntdll -I\$(T)/include"
  echo "PATHS := -DBINDIR='\"/wine/bin\"' -DLIBDIR='\"/wine/lib\"' -DDATADIR='\"/wine/share\"' -DSYSTEMDLLPATH='\"\"'"
  echo "SERVER := -D__WINESRC__ -I\$(W)/include -I\$(T)/server -I\$(T)/include -I\$(W)/src -include \$(W)/rename-server.h"
  # win32u's EXTRADEFS (dlls/win32u/Makefile.in L1), and its renames.
  echo "WIN32U := -D__WINESRC__ -D_WIN32U_ -I\$(W)/include -I\$(T)/dlls/win32u -I\$(T)/include -I$rel/gen/include -I\$(W)/src -include \$(W)/rename-win32u.h"
  echo "OWN := \$(NTDLL) -I\$(T)/dlls/ntdll/unix -Wall"
  echo "LDFLAGS := -m64 -O2 -sIMPORTED_MEMORY -sSHARED_MEMORY -sALLOW_MEMORY_GROWTH -sINITIAL_MEMORY=4GB \\"
  echo "  -sMAXIMUM_MEMORY=16GB -sGLOBAL_BASE=2GB -fwasm-exceptions -sSUPPORT_LONGJMP=wasm \\"
  echo "  -sMODULARIZE -sEXPORT_ES6 -sENVIRONMENT=node,worker --no-entry \\"
  echo "  -sSTACK_SIZE=1MB -sSTACK_OVERFLOW_CHECK=2 -Wl,--wrap=sbrk -Wl,--fatal-warnings -Wl,--no-gc-sections -Wl,--error-limit=0 \\"
  echo "  -sEXPORTED_RUNTIME_METHODS=FS,ENV --js-library \$(W)/src/library.js"
  objs=()
  for f in $ntdll_srcs; do
    o="\$(O)/ntdll/${f%.c}.o"; objs+=("$o")
    extra=""; [ "$f" = loader.c ] && extra=" \$(PATHS)"
    printf '%s: $(T)/dlls/ntdll/unix/%s $(MK)\n\temcc $(CFLAGS) $(NTDLL)%s -c -o $@ $<\n' "$o" "$f" "$extra"
  done
  for f in $server_srcs; do
    o="\$(O)/server/${f%.c}.o"; objs+=("$o")
    extra=""; [ "$f" = unicode.c ] && extra=" \$(PATHS)"
    printf '%s: $(T)/server/%s $(MK)\n\temcc $(CFLAGS) $(SERVER)%s -c -o $@ $<\n' "$o" "$f" "$extra"
  done
  for f in $win32u_srcs; do
    o="\$(O)/win32u/${f%.c}.o"; objs+=("$o")
    printf '%s: $(T)/dlls/win32u/%s $(MK)\n\temcc $(CFLAGS) $(WIN32U) -c -o $@ $<\n' "$o" "$f"
  done
  for f in $own_srcs; do
    o="\$(O)/own/${f%.c}.o"; objs+=("$o")
    printf '%s: $(W)/src/%s $(MK)\n\temcc $(CFLAGS) $(OWN) -c -o $@ $<\n' "$o" "$f"
  done
  o="\$(O)/gen/syscall_table.o"; objs+=("$o")
  printf '%s: %s/gen/syscall_table.c $(MK)\n\temcc $(CFLAGS) $(OWN) -c -o $@ $<\n' "$o" "$rel"
  echo "OBJS := ${objs[*]}"
  echo "SERVER_OBJS := \$(filter \$(O)/server/%,\$(OBJS))"
  echo "objs: \$(OBJS)$(for t in $tests; do printf ' $(O)/tests/%s.o' "$t"; done)"
  echo "all: $rel/wine-unix.mjs$(for t in $tests; do printf ' %s/test-%s.mjs' "$rel" "$t"; done)"
  printf '%s/wine-unix.mjs: $(OBJS) $(W)/src/library.js $(MK)\n\temcc $(LDFLAGS) -sEXPORTED_FUNCTIONS=_boot,%s -o $@ $(OBJS)\n' "$rel" "$exports_common"
  for t in $tests; do
    printf '$(O)/tests/%s.o: $(W)/tests/%s.c $(MK)\n\temcc $(CFLAGS) $(OWN) -I$(W)/tests -I$(W)/src -c -o $@ $<\n' "$t" "$t"
    printf '%s/test-%s.mjs: $(OBJS) $(O)/tests/%s.o $(W)/src/library.js $(MK)\n\temcc $(LDFLAGS) -sEXPORTED_FUNCTIONS=_tw_test_main,%s -o $@ $(OBJS) $(O)/tests/%s.o\n' \
      "$rel" "$t" "$t" "$exports_common" "$t"
  done
  echo "-include \$(OBJS:.o=.d) \$(wildcard \$(O)/tests/*.d)"
  echo ".DEFAULT_GOAL := all"
} > "$mk.new"
# Rewritten only when it changes: every object depends on it (the flags).
cmp -s "$mk.new" "$mk" && rm "$mk.new" || mv "$mk.new" "$mk"

nsrc=$(( $(echo $ntdll_srcs | wc -w) + $(echo $server_srcs | wc -w) + $(echo $win32u_srcs | wc -w) + $(echo $own_srcs | wc -w) + 1 ))
run_make() { # TARGET: make inside the Emscripten container, logged to obj/make.log
  set +e
  "$emcc_sh" make --no-print-directory -k -j"$jobs" -f "$rel/obj/build.mk" "$1" >> "$out/obj/make.log" 2>&1
  local st=$?
  set -e
  return $st
}
: > "$out/obj/make.log"
log "compiling $nsrc sources with $jobs jobs (log: $rel/obj/make.log)"
if ! run_make objs; then
  grep -E 'error' "$out/obj/make.log" | head -n 60 >&2 || true
  die "compile failed (full log: $rel/obj/make.log)"
fi

# ---- 5. Gates ----------------------------------------------------------------
# Gate 3, before the link (which would only report a duplicate symbol):
# global symbols defined by two of the three parts, the server's objects,
# ntdll's (with src/ and the syscall wrappers) and win32u's.
# rename-server.h renames the server's NAME to wineserver_NAME and
# rename-win32u.h win32u's to win32u_NAME; ntdll keeps its names, since
# win32u calls ntdll's exports by name. Stripping the prefixes recovers the
# original names for the census.
"$emcc_sh" emnm -g --defined-only "$obj"/server/*.o > "$out/obj/server.nm" 2>/dev/null
"$emcc_sh" emnm -g --defined-only "$obj"/ntdll/*.o "$obj"/own/*.o "$obj"/gen/*.o > "$out/obj/ntdll.nm" 2>/dev/null
"$emcc_sh" emnm -g --defined-only "$obj"/win32u/*.o "$obj"/win32u/dibdrv/*.o > "$out/obj/win32u.nm" 2>/dev/null
syms() { awk 'NF==3 && $2 != "U" {print $3}' "$1" | sort -u; }
for part in server ntdll win32u; do syms "$out/obj/$part.nm" > "$out/obj/$part.syms"; done
sed 's/^wineserver_//' "$out/obj/server.syms" | sort -u > "$out/obj/server.orig"
sed 's/^win32u_//' "$out/obj/win32u.syms" | sort -u > "$out/obj/win32u.orig"
cp "$out/obj/ntdll.syms" "$out/obj/ntdll.orig"
: > "$out/obj/unrenamed.txt"
ncoll=0
for pair in "server ntdll rename-server.h" "win32u ntdll rename-win32u.h" "win32u server rename-win32u.h"; do
  read -r a b hdr <<< "$pair"
  comm -12 "$out/obj/$a.orig" "$out/obj/$b.orig" > "$out/obj/collisions-$a-$b.txt"
  ncoll=$(( ncoll + $(wc -l < "$out/obj/collisions-$a-$b.txt") ))
  if [ $census -eq 1 ]; then
    log "census: $(wc -l < "$out/obj/collisions-$a-$b.txt") global symbols defined by both $a and $b (renamed in $hdr):"
    cat "$out/obj/collisions-$a-$b.txt"
  fi
  comm -12 "$out/obj/$a.syms" "$out/obj/$b.syms" | sed "s|\$| ($a and $b: rename it in native/wine/$hdr)|" >> "$out/obj/unrenamed.txt"
done
if [ -s "$out/obj/unrenamed.txt" ]; then
  echo "build.sh: gate 3 (collisions) failed: these globals are defined by two parts (--census lists all):" >&2
  sed 's/^/  /' "$out/obj/unrenamed.txt" >&2
  exit 1
fi
log "gate 3: $ncoll collisions, all renamed by rename-server.h and rename-win32u.h"

# Gate 1: the link keeps ERROR_ON_UNDEFINED_SYMBOLS=1, with no section GC.
log "linking"
if ! run_make all; then
  grep -E 'error|undefined symbol' "$out/obj/make.log" | head -n 60 >&2 || true
  if grep -q 'undefined symbol' "$out/obj/make.log"; then
    echo "build.sh: gate 1 (no undefined symbols) failed: implement each symbol where wine-unix.md says," \
      "or give it a stub in native/wine/src/stubs.c (section 2)" >&2
  fi
  die "link failed (full log: $rel/obj/make.log)"
fi
log "gate 1: linked with no undefined symbols"

# Gate 2: every import is listed.
modules=("$out/wine-unix.wasm")
for t in $tests; do modules+=("$out/test-$t.wasm"); done
node --input-type=module - "$wn/imports.txt" "${modules[@]}" > "$out/obj/imports.json" <<'EOF'
// Gate 2: every import of every module (module.field, from its import
// section) is listed in imports.txt.
import { readFileSync } from 'node:fs';
const [listFile, ...wasms] = process.argv.slice(2);
const listed = new Set(readFileSync(listFile, 'utf8').split('\n')
  .map((l) => l.replace(/#.*/, '').trim()).filter(Boolean).map((l) => l.split(/\s+/)[0]));
const all = new Set();
let bad = 0;
for (const w of wasms) {
  const imps = WebAssembly.Module.imports(new WebAssembly.Module(readFileSync(w)));
  for (const { module, name } of imps) {
    const key = `${module}.${name}`;
    all.add(key);
    if (!listed.has(key)) { console.error(`gate 2: ${w.replace(/.*\//, '')} imports ${key}, which imports.txt doesn't list: add it with its reason`); bad++; }
  }
}
if (bad) process.exit(1);
console.log(JSON.stringify([...all].sort()));
EOF
log "gate 2: $(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1])).length)' "$out/obj/imports.json") imports, all listed in imports.txt"

# ---- 6. build-info.json -------------------------------------------------------
emver="$("$emcc_sh" emcc --version 2>/dev/null | head -n 1)"
raw=$(stat -c %s "$out/wine-unix.wasm")
gz=$(gzip -9 -c "$out/wine-unix.wasm" | wc -c)
node - "$out/build-info.json" "$emver" "$sha256" "$raw" "$gz" "$out/obj/imports.json" \
  ${patches[@]+"${patches[@]}"} <<'EOF'
const fs = require('fs'), crypto = require('crypto');
const [file, emcc, sha, raw, gz, imports, ...patches] = process.argv.slice(2);
const info = {
  emcc,
  wine_tarball_sha256: sha,
  patches: patches.map((p) => ({ name: p.replace(/.*\//, ''),
    sha256: crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex') })),
  imports: JSON.parse(fs.readFileSync(imports, 'utf8')),
  wasm_bytes: Number(raw),
  wasm_gzip_bytes: Number(gz),
};
fs.writeFileSync(file, JSON.stringify(info, null, 2) + '\n');
EOF
log "done in $(( $(date +%s) - start )) s: $rel/wine-unix.wasm $raw bytes ($gz gzipped)$(for t in $tests; do printf ', test-%s' "$t"; done)"
