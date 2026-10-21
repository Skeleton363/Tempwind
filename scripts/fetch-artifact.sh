#!/usr/bin/env bash
# Download a release asset of skeleton363/tempwind.
#   scripts/fetch-artifact.sh TAG ASSET [DIR]
#   scripts/fetch-artifact.sh --latest PREFIX ASSET [DIR]   (newest release tagged PREFIX-*)
# DIR defaults to target/artifacts. If ASSET.sha256 exists in the release it is
# downloaded and verified. Prints the path of the downloaded asset.
set -euo pipefail

REPO="skeleton363/tempwind"

usage() {
  echo "usage: $0 TAG ASSET [DIR] | $0 --latest PREFIX ASSET [DIR]" >&2
  exit 2
}

curl_retry() { # url out ; retries 4 times with backoff 2/4/8/16 s
  local url=$1 out=$2 delay=2 attempt code
  for attempt in 1 2 3 4 5; do
    code=$(curl -sSL -w '%{http_code}' "$url" -o "$out" || true)
    if [ "$code" = 200 ]; then return 0; fi
    rm -f "$out"
    # A definite 404 will not fix itself; do not wait.
    if [ "$code" = 404 ]; then echo "fetch-artifact: 404 for $url" >&2; return 1; fi
    [ "$attempt" -lt 5 ] || break
    echo "fetch-artifact: attempt $attempt failed for $url; retrying in ${delay}s" >&2
    sleep "$delay"
    delay=$((delay * 2))
  done
  return 1
}

fetch() { # tag name out ; returns non-zero if unavailable
  local tag=$1 name=$2 out=$3
  if curl_retry "https://github.com/$REPO/releases/download/$tag/$name" "$out"; then return 0; fi
  if command -v gh >/dev/null 2>&1; then
    echo "fetch-artifact: curl failed, trying gh for $name" >&2
    local tmp
    tmp=$(mktemp -d)
    if gh release download "$tag" -R "$REPO" -p "$name" -D "$tmp" --clobber 2>/dev/null; then
      mv "$tmp/$name" "$out"
      rmdir "$tmp"
      return 0
    fi
    rm -rf "$tmp"
    # gh release download needs GraphQL, which some sandboxes block; use REST.
    local id
    id=$(gh api "repos/$REPO/releases/tags/$tag" --jq ".assets[] | select(.name==\"$name\") | .id" 2>/dev/null || true)
    case $id in ""|*[!0-9]*) id= ;; esac
    if [ -n "$id" ] && gh api -H 'Accept: application/octet-stream' "repos/$REPO/releases/assets/$id" >"$out"; then
      return 0
    fi
    rm -f "$out"
  fi
  return 1
}

latest_tag() { # prefix
  local prefix=$1 json delay=2 attempt
  for attempt in 1 2 3 4 5; do
    if json=$(curl -fsSL -H 'Accept: application/vnd.github+json' \
      ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
      "https://api.github.com/repos/$REPO/releases?per_page=100"); then
      # Releases are returned newest first; take the first matching tag.
      printf '%s\n' "$json" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/' |
        grep -m1 "^${prefix}-" && return 0
      echo "fetch-artifact: no release with tag prefix ${prefix}-" >&2
      return 1
    fi
    [ "$attempt" -lt 5 ] || break
    sleep "$delay"
    delay=$((delay * 2))
  done
  if command -v gh >/dev/null 2>&1; then
    gh release list -R "$REPO" --limit 100 --json tagName -q '.[].tagName' | grep -m1 "^${prefix}-" && return 0
  fi
  echo "fetch-artifact: could not list releases" >&2
  return 1
}

if [ "${1:-}" = "--latest" ]; then
  [ $# -ge 3 ] && [ $# -le 4 ] || usage
  tag=$(latest_tag "$2")
  asset=$3
  dir=${4:-target/artifacts}
else
  [ $# -ge 2 ] && [ $# -le 3 ] || usage
  tag=$1
  asset=$2
  dir=${3:-target/artifacts}
fi

mkdir -p "$dir"
fetch "$tag" "$asset" "$dir/$asset" || { echo "fetch-artifact: failed to download $asset from $tag" >&2; exit 1; }

sha="$dir/$asset.sha256"
if fetch "$tag" "$asset.sha256" "$sha" 2>/dev/null; then
  expected=$(awk '{print $1}' "$sha")
  actual=$(sha256sum "$dir/$asset" | awk '{print $1}')
  if [ "$expected" != "$actual" ]; then
    echo "fetch-artifact: sha256 mismatch for $asset (expected $expected, got $actual)" >&2
    exit 1
  fi
  echo "fetch-artifact: sha256 ok" >&2
else
  rm -f "$sha"
  echo "fetch-artifact: no $asset.sha256 in release; not verified" >&2
fi
echo "$dir/$asset"
