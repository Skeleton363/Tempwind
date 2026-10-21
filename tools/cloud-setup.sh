#!/bin/bash
# shellcheck disable=SC2329  # step functions run indirectly, via declare -f
# Tempwind cloud environment setup script.
#
# Paste this whole file into the environment's settings (cloud environment
# menu in the session's title bar -> Edit -> Setup script). It runs once
# when the environment is built; its results are cached as files, so it never
# leaves a process running (dockerd is started per session by the
# SessionStart hook in .claude/settings.json).
#
# Installs: MinGW-w64 x86-64 (gcc, g++), wabt, binaryen, the pinned Rust
# toolchain with the wasm32 target, Node 24 from nodejs.org (made the default
# `node`), and pre-pulls the emscripten/emsdk image. Chrome is skipped:
# Playwright's Chromium is already in the image.
#
# Budget: under 5 minutes. Always exits 0; each step logs OK / FAILED /
# SKIPPED, with details in $LOG. Safe to run again.
#
# Keep these in sync with the repository:
RUST_TOOLCHAIN=1.97.0                # rust-toolchain.toml
NODE_VERSION=24.22.0                 # .nvmrc
EMSDK_IMAGE=emscripten/emsdk:6.0.12  # scripts/emcc.sh

BUDGET=250   # seconds for the timed steps; stopping dockerd and the summary fit in the rest of 5 minutes
LOG=/tmp/tempwind-cloud-setup.log
NODE_DIR=/opt/node24

export RUST_TOOLCHAIN NODE_VERSION EMSDK_IMAGE NODE_DIR
export DEBIAN_FRONTEND=noninteractive
export PATH="$HOME/.cargo/bin:$PATH"
SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"
export SUDO

: > "$LOG"
start=$SECONDS
results=()

# step NAME MAX_SECONDS FUNCTION: run FUNCTION under a timeout capped by the
# remaining budget; never stops the script.
step() {
  local name=$1 max=$2 fn=$3 left t0 rc
  left=$(( BUDGET - (SECONDS - start) ))
  if (( left < 10 )); then
    results+=("SKIPPED  $name (out of time)")
    return
  fi
  (( max > left )) && max=$left
  t0=$SECONDS
  echo "=== $name (limit ${max}s)" >> "$LOG"
  timeout --kill-after=10 "$max" bash -c "set -euo pipefail; $(declare -f "$fn"); $fn" >> "$LOG" 2>&1
  rc=$?
  if (( rc == 0 )); then
    results+=("OK       $name ($(( SECONDS - t0 ))s)")
  elif (( rc == 124 || rc == 137 )); then
    results+=("FAILED   $name (timed out after ${max}s)")
  else
    results+=("FAILED   $name (exit $rc)")
  fi
}

apt_packages() {
  local pkgs=(gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 wabt binaryen)
  if dpkg -s "${pkgs[@]}" >/dev/null 2>&1; then
    echo "already installed: ${pkgs[*]}"
    return 0
  fi
  $SUDO apt-get update -qq || echo "apt-get update failed; trying the cached package lists"
  $SUDO apt-get install -y -qq --no-install-recommends "${pkgs[@]}"
}

rust_toolchain() {
  rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal \
    --component rustfmt --component clippy --target wasm32-unknown-unknown
  rustup target add wasm32-unknown-unknown   # the default toolchain too
}

node24() {
  local name="node-v$NODE_VERSION-linux-x64"
  local base="https://nodejs.org/dist/v$NODE_VERSION"
  if [ "$("$NODE_DIR/bin/node" --version 2>/dev/null)" != "v$NODE_VERSION" ]; then
    local tmp
    tmp=$(mktemp -d)
    curl -fsSL --retry 4 --retry-delay 2 -o "$tmp/$name.tar.xz" "$base/$name.tar.xz"
    curl -fsSL --retry 4 --retry-delay 2 -o "$tmp/SHASUMS256.txt" "$base/SHASUMS256.txt"
    (cd "$tmp" && grep " $name.tar.xz\$" SHASUMS256.txt | sha256sum -c -)
    $SUDO rm -rf "$NODE_DIR"
    $SUDO mkdir -p "$NODE_DIR"
    $SUDO tar -xJf "$tmp/$name.tar.xz" -C "$NODE_DIR" --strip-components=1
    rm -rf "$tmp"
  fi
  # Make Node 24 the default `node`. Non-login shells: ~/.local/bin comes
  # before the image's /opt/node22/bin on PATH, and /usr/local/bin covers
  # other PATHs. Login shells: the image's /etc/profile.d/nodejs.sh prepends
  # /opt/node22/bin, so a later profile.d script prepends Node 24.
  mkdir -p "$HOME/.local/bin"
  local tool
  for tool in node npm npx corepack; do
    ln -sfn "$NODE_DIR/bin/$tool" "$HOME/.local/bin/$tool"
    $SUDO ln -sfn "$NODE_DIR/bin/$tool" "/usr/local/bin/$tool"
  done
  echo "export PATH=$NODE_DIR/bin:\$PATH" | $SUDO tee /etc/profile.d/zz-node24.sh >/dev/null
  PATH="$HOME/.local/bin:$PATH" node --version
  bash -lc 'node --version'
}

# Docker: start dockerd directly (the init script fails on a ulimit error in
# this sandbox), wait until it answers, pull the image. docker_stop then runs
# unconditionally, outside any timeout, so no live-looking pid file or socket
# ends up in the cached environment.
docker_start() {
  if docker info >/dev/null 2>&1; then
    echo "dockerd already answering"
    return 0
  fi
  $SUDO rm -f /var/run/docker.pid /var/run/docker/containerd/containerd.pid
  $SUDO setsid -f dockerd > /tmp/dockerd-setup.log 2>&1 < /dev/null
  for _ in $(seq 1 60); do
    if docker info >/dev/null 2>&1; then
      docker info --format 'docker {{.ServerVersion}} answering'
      return 0
    fi
    sleep 1
  done
  echo "dockerd did not answer within 60 s"
  return 1
}

docker_pull() {
  docker pull --quiet "$EMSDK_IMAGE"
}

docker_stop() {
  local p c
  p=$(cat /var/run/docker.pid 2>/dev/null || true)
  c=$(cat /var/run/docker/containerd/containerd.pid 2>/dev/null || true)
  if [ -n "$p" ]; then $SUDO kill -TERM "$p" 2>/dev/null || true; fi
  for _ in $(seq 1 20); do
    pgrep -x dockerd >/dev/null || break
    sleep 1
  done
  if pgrep -x dockerd >/dev/null; then $SUDO pkill -KILL -x dockerd || true; fi
  # A clean dockerd shutdown stops its managed containerd; after a KILL it can
  # linger. Use its pid file: a `pkill -f` pattern would match this shell.
  if [ -n "$c" ] && kill -0 "$c" 2>/dev/null; then
    $SUDO kill -TERM "$c" 2>/dev/null || true
    sleep 2
  fi
  $SUDO rm -f /var/run/docker.pid /var/run/docker.sock /var/run/docker/containerd/containerd.pid
  if pgrep -x dockerd >/dev/null || { [ -n "$c" ] && kill -0 "$c" 2>/dev/null; }; then
    echo "dockerd or its containerd is still running"
    return 1
  fi
  echo "dockerd stopped, pid files removed"
}

step "apt: MinGW-w64 x86-64, wabt, binaryen" 120 apt_packages
step "rust $RUST_TOOLCHAIN + wasm32-unknown-unknown" 90 rust_toolchain
step "node v$NODE_VERSION from nodejs.org" 90 node24
step "docker: start dockerd" 70 docker_start
step "docker: pull $EMSDK_IMAGE" 200 docker_pull
# Always stop dockerd, whatever happened above; not limited by BUDGET.
echo "=== docker: stop dockerd" >> "$LOG"
if timeout --kill-after=5 40 bash -c "$(declare -f docker_stop); docker_stop" >> "$LOG" 2>&1; then
  results+=("OK       docker: stop dockerd")
else
  results+=("FAILED   docker: stop dockerd (see $LOG)")
fi

echo "Tempwind cloud setup finished in $(( SECONDS - start ))s (log: $LOG)"
printf '  %s\n' "${results[@]}"
{
  PATH="$HOME/.local/bin:$PATH" node --version 2>/dev/null | sed 's/^/  node /'
  x86_64-w64-mingw32-gcc --version 2>/dev/null | head -1 | sed 's/^/  /'
  wat2wasm --version 2>/dev/null | sed 's/^/  wabt /'
  wasm-opt --version 2>/dev/null | sed 's/^/  /'
  rustup run "$RUST_TOOLCHAIN" rustc --version 2>/dev/null | sed 's/^/  /'
} || true
exit 0
