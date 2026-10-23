#!/usr/bin/env bash
# Install apt packages on a GitHub runner, retrying hung or failed attempts.
#   tests/targets/apt-install.sh PACKAGE...
# A mirror that stalls can hang apt-get for a quarter of an hour (seen twice
# in targets.yml), so each attempt gets its own time limit and is retried.
set -euo pipefail

attempts=3
limit=240 # seconds per apt-get call

apt() {
  timeout --kill-after=10 "$limit" sudo apt-get \
    -o Acquire::Retries=3 -o Acquire::http::Timeout=30 "$@"
}

for attempt in $(seq "$attempts"); do
  if apt update && apt install -y --no-install-recommends "$@"; then
    exit 0
  fi
  echo "::warning::apt-get attempt $attempt of $attempts failed for: $*"
  sleep $((attempt * 10))
done
echo "::error::could not install: $*"
exit 1
