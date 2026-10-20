#!/usr/bin/env bash
# Run emcc (or another emsdk tool) inside the pinned Emscripten Docker image.
#
#   scripts/emcc.sh [TOOL] ARGS...
#
# TOOL is optional. If the first argument names a tool (emcc, em++, emar,
# emranlib, emconfigure, emmake, emcmake, emnm, emsize, emstrip, node,
# wasm-*, llvm-*, clang, clang++, python3, bash, sh, make, cmake), that tool
# runs; otherwise ARGS go to emcc. Use `scripts/emcc.sh emcc --version` if the
# first emcc argument could be mistaken for a tool name.
#
# - The repository is mounted at /src. The working directory is /src, or the
#   matching directory under /src when called from a subdirectory of the
#   repository, so relative paths in ARGS keep working.
# - The container runs as the calling uid:gid, so outputs are owned by you.
# - The Emscripten cache (system libraries built on first use) persists in
#   EMCC_CACHE_DIR (default: ${XDG_CACHE_HOME:-~/.cache}/tempwind/emcache-<tag>)
#   so later runs skip rebuilding libc.
#
# Environment:
#   EMSDK_IMAGE       image to use (default emscripten/emsdk:6.0.12)
#   EMCC_CACHE_DIR    host directory for the Emscripten cache
#   EMCC_DOCKER_ARGS  extra arguments for `docker run` (word-split)
#   EMCC_CFLAGS, EMCC_DEBUG, EMCC_FORCE_STDLIBS, EM_* are passed through.
set -euo pipefail

image="${EMSDK_IMAGE:-emscripten/emsdk:6.0.12}"

if ! command -v docker >/dev/null 2>&1; then
  echo "emcc.sh: docker not found. Install Docker or run inside an emsdk environment." >&2
  exit 127
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cwd="$(pwd -P)"
case "$cwd/" in
  "$repo_root"/*) workdir="/src${cwd#"$repo_root"}" ;;
  *) workdir="/src" ;;
esac

tool="emcc"
if [ $# -gt 0 ]; then
  case "$1" in
    em++|emcc|emar|emranlib|emconfigure|emmake|emcmake|emnm|emsize|emstrip|emdump|\
    node|npm|npx|clang|clang++|python3|bash|sh|make|cmake|wasm-*|llvm-*)
      tool="$1"; shift ;;
  esac
fi

tag="${image##*:}"
[ "$tag" = "$image" ] && tag="latest"
cache_dir="${EMCC_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/tempwind/emcache-${tag//[^A-Za-z0-9._-]/_}}"
mkdir -p "$cache_dir"

tty_args=(-i)
if [ -t 0 ] && [ -t 1 ]; then tty_args=(-it); fi

env_args=()
for v in $(compgen -e); do
  case "$v" in
    EM_CACHE|EM_CONFIG) ;;
    EMCC_CFLAGS|EMCC_DEBUG|EMCC_FORCE_STDLIBS|EMCC_SKIP_SANITY_CHECK|EM_*) env_args+=(-e "$v") ;;
  esac
done

# shellcheck disable=SC2206
extra_args=(${EMCC_DOCKER_ARGS:-})

# The image's entrypoint sources emsdk_env.sh, which unsets EM_CACHE, so we
# replace it with one that sources emsdk_env.sh and then sets EM_CACHE.
# shellcheck disable=SC2016
entry='EMSDK_QUIET=1 . /emsdk/emsdk_env.sh >/dev/null 2>&1; export EM_CACHE=/emcache; exec "$@"'

exec docker run --rm "${tty_args[@]}" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  "${env_args[@]}" \
  -v "$repo_root:/src" \
  -v "$cache_dir:/emcache" \
  -w "$workdir" \
  "${extra_args[@]}" \
  --entrypoint /bin/bash \
  "$image" \
  -c "$entry" emcc.sh "$tool" "$@"
