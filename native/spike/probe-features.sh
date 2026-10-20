#!/usr/bin/env bash
# Compile each PROBE_* section of features.c with the given compiler and
# print one JSON object {"PROBE": true|false, ...}. Called by run.sh, once on
# the host with gcc and once inside the emsdk container with emcc.
#
#   probe-features.sh OUTDIR CC [FLAGS...]
set -uo pipefail
outdir="$1"; shift
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$here/features.c"
mkdir -p "$outdir"

probes=$(grep -o 'defined(PROBE_[A-Z0-9_]*)' "$src" | sed 's/defined(\(.*\))/\1/')

printf '{'
sep=''
for p in $probes; do
  log="$outdir/$p.log"
  case "$p" in
    PROBE_LINK_*) args=("$src" -o "$outdir/$p.out") ;;
    *)            args=(-c "$src" -o "$outdir/$p.o") ;;
  esac
  if "$@" -Werror -D"$p" "${args[@]}" >"$log" 2>&1; then r=true; else r=false; fi
  printf '%s\n  "%s": %s' "$sep" "${p#PROBE_}" "$r"
  sep=','
done
printf '\n}\n'
