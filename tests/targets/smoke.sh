#!/usr/bin/env bash
# Smoke test of the Phase 1 targets: 7zz.exe and coremark.exe.
#   tests/targets/smoke.sh DIR
# DIR holds 7zz.exe and coremark.exe. Runs on Windows (Git Bash, executables
# started directly) and on Linux, where RUN names the launcher:
#   RUN=wine64 tests/targets/smoke.sh DIR
# Every step runs in a scratch directory with relative paths, so Git Bash's
# path conversion and Wine's drive mapping don't matter. Stops at the first
# failing step and says which one.
set -Eeuo pipefail

[ $# -eq 1 ] || { echo "usage: $0 DIR" >&2; exit 2; }
dir=$(cd "$1" && pwd)
run_prefix=${RUN:-}
z7="$dir/7zz.exe"
coremark="$dir/coremark.exe"
[ -f "$z7" ] && [ -f "$coremark" ] || { echo "smoke: 7zz.exe or coremark.exe missing in $dir" >&2; exit 2; }

step=
trap 'echo "::error::smoke test failed at step: $step"; echo "smoke: FAILED at step: $step" >&2' ERR

# exe ARGS... : run an executable through the launcher.
exe() {
  # shellcheck disable=SC2086
  $run_prefix "$@"
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

step="7zz i"
exe "$z7" i > i.out
grep -q "7z" i.out
echo "ok: $step"

# A small tree: text, a compressible and an incompressible binary file, a nested
# directory, an empty file.
mkdir -p tree/sub/deeper
printf 'hello\nworld\n' > tree/hello.txt
seq 1 20000 > tree/numbers.txt
head -c 70000 /dev/urandom > tree/random.bin
: > tree/empty
printf 'nested file\n' > tree/sub/deeper/nested.txt
printf 'second\n' > tree/sub/other.txt

for fmt in 7z zip; do
  step="7zz a ($fmt)"
  exe "$z7" a -mmt1 -bsp0 "t.$fmt" tree > "a-$fmt.out"
  step="7zz t ($fmt)"
  exe "$z7" t -bsp0 "t.$fmt" > "t-$fmt.out"
  grep -q "Everything is Ok" "t-$fmt.out"
  step="7zz x ($fmt)"
  exe "$z7" x -bsp0 -y "-oout-$fmt" "t.$fmt" > "x-$fmt.out"
  step="compare extracted tree ($fmt)"
  diff -r tree "out-$fmt/tree"
  echo "ok: 7zz round trip, $fmt"
done

# Iteration count small enough to finish in a second or two; the "at least 10
# secs" complaint is expected and ignored.
step="coremark 0x0 0x0 0x66 200"
exe "$coremark" 0x0 0x0 0x66 200 > coremark.out || true
cat coremark.out
grep -q "Correct operation validated" coremark.out
echo "ok: $step"

step=
echo "smoke: all steps passed"
