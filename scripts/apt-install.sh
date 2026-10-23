#!/usr/bin/env bash
# Install apt packages on a GitHub runner, retrying stalled or slow downloads.
#   scripts/apt-install.sh PACKAGE...
# The runners fetch from azure.archive.ubuntu.com first (mirror+file:
# /etc/apt/apt-mirrors.txt). That mirror has trickled at about 40 KB/s for
# many minutes, hanging jobs, so every attempt gets a time limit, and the
# retries drop the Azure mirror in favour of the next ones in the list.
set -euo pipefail

mirrors=${APT_MIRRORS_FILE:-/etc/apt/apt-mirrors.txt}
# Seconds per attempt (apt-get update + install); overridable for tests.
read -r -a limits <<< "${APT_ATTEMPT_LIMITS:-300 600 600}"

for attempt in 1 2 3; do
  if [ "$attempt" -gt 1 ]; then
    # A killed attempt may have interrupted dpkg.
    sudo dpkg --configure -a || true
    if [ -f "$mirrors" ] && grep -q azure "$mirrors"; then
      echo "apt-install: retrying without the Azure mirror"
      grep -v azure "$mirrors" | sudo tee "$mirrors.new" >/dev/null
      sudo mv "$mirrors.new" "$mirrors"
      cat "$mirrors"
    fi
  fi
  limit=${limits[$((attempt - 1))]}
  if timeout --kill-after=10 "$limit" bash -c '
      set -e
      opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30)
      sudo apt-get "${opts[@]}" update
      sudo apt-get "${opts[@]}" install -y --no-install-recommends "$@"
    ' apt-install "$@"; then
    exit 0
  fi
  echo "::warning::apt-get attempt $attempt of 3 (limit ${limit}s) failed for: $*"
done
echo "::error::could not install: $*"
exit 1
