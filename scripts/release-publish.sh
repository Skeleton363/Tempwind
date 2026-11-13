#!/usr/bin/env bash
# Publish files to the release TAG of this repository, creating it if needed.
#
#   SHA=<commit> scripts/release-publish.sh TAG NOTES FILE...
#
# - The release is created at $SHA as a prerelease, unless it exists already.
# - Each FILE goes up with its FILE.sha256 (written next to it), unless the
#   release already has an asset of that name: assets are never replaced
#   (docs/design/wine-build.md, "Release"). A FILE already there without its
#   FILE.sha256 (an earlier attempt stopped between the two uploads) gets one
#   made from the release's copy, which another build may have uploaded.
# - Every GitHub API call is retried with a growing pause. The releases API
#   has refused a run's GITHUB_TOKEN twice with a transient "HTTP 403: Resource
#   not accessible by integration": the Phase 1 targets run of 6d4536c, and the
#   publish job of wine run 42353561868. Both times the same call succeeded a
#   little later.
#
# Needs gh with GH_TOKEN, and sha256sum.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: SHA=<commit> $0 TAG NOTES [FILE...]" >&2
  exit 2
fi
tag=$1 notes=$2
shift 2
: "${SHA:?SHA must name the commit of the release}"
pause=${RELEASE_RETRY_PAUSE:-15}

retry() { # command...
  local i
  for i in 1 2 3 4 5; do
    if "$@"; then return 0; fi
    echo "release-publish: attempt $i of '$*' failed" >&2
    [ "$i" -lt 5 ] && sleep $((i * pause))
  done
  return 1
}

release_exists() { gh release view "$tag" >/dev/null 2>&1; }
create_release() {
  release_exists || gh release create "$tag" --target "$SHA" --prerelease --title "$tag" --notes "$notes"
}
asset_state() { # name -> prints "present" or "missing"; fails only if the listing fails
  local names
  names=$(gh release view "$tag" --json assets --jq '.assets[].name') || return 1
  if grep -qxF "$1" <<<"$names"; then echo present; else echo missing; fi
}
upload_one() { # file: uploads it unless the release has it (an earlier attempt may have)
  local state
  state=$(asset_state "$(basename "$1")") || return 1
  [ "$state" = present ] || gh release upload "$tag" "$1"
}

checksum_of_release_copy() { # name dir: the release's copy of name and its .sha256, in dir
  gh release download "$tag" --pattern "$1" --dir "$2" --clobber &&
    (cd "$2" && sha256sum "$1" > "$1.sha256")
}

retry create_release
for f in "$@"; do
  name=$(basename "$f")
  state=$(retry asset_state "$name")
  if [ "$state" = missing ]; then
    (cd "$(dirname "$f")" && sha256sum "$name" > "$name.sha256")
    retry upload_one "$f"
    retry upload_one "$f.sha256"
    echo "release-publish: uploaded $name to $tag"
    continue
  fi
  sum_state=$(retry asset_state "$name.sha256")
  if [ "$sum_state" = present ]; then
    echo "release-publish: $name is already on $tag; kept"
    continue
  fi
  dir=$(mktemp -d)
  retry checksum_of_release_copy "$name" "$dir"
  retry upload_one "$dir/$name.sha256"
  rm -rf "$dir"
  echo "release-publish: $name was on $tag without its checksum; added $name.sha256 from the release's copy"
done
