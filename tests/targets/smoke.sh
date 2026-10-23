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

# A run of 200 iterations takes milliseconds. CoreMark then reports "ERROR! Must
# execute for at least 10 secs" and "Errors detected" instead of "Correct
# operation validated", whatever the results. So check what CoreMark validates
# itself: the run type it recognizes (2K performance run, seeds 0 0 0x66) and the
# four CRCs it compares with its known values; a wrong CRC prints an extra
# "ERROR! ... crc" line.
step="coremark 0x0 0x0 0x66 200"
exe "$coremark" 0x0 0x0 0x66 200 > coremark.out || true
cat coremark.out
grep -q "2K performance run parameters for coremark" coremark.out
grep -q "seedcrc          : 0xe9f5" coremark.out
grep -q "\[0\]crclist       : 0xe714" coremark.out
grep -q "\[0\]crcmatrix     : 0x1fd7" coremark.out
grep -q "\[0\]crcstate      : 0x8e3a" coremark.out
if grep -q "ERROR! .* crc" coremark.out; then
  echo "coremark reported a wrong crc" >&2
  false
fi
# The only complaint allowed is the 10 second one.
[ "$(grep -c 'ERROR' coremark.out)" -eq 1 ]
grep -q "ERROR! Must execute for at least 10 secs" coremark.out
echo "ok: $step"

step=
echo "smoke: all steps passed"
