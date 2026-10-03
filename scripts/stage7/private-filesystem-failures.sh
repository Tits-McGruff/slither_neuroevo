#!/usr/bin/env bash
# Run actual ENOSPC archive regressions in an unprivileged, private mount namespace.
# Requires Linux, user namespaces, mount, and the source-matched production addon.
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  printf '%s\n' 'This acceptance fixture requires Linux user and mount namespaces.' >&2
  exit 1
fi

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd -- "$repo_root"
mkdir -p -- "$repo_root/data"
fixture_root=$(mktemp -d "$repo_root/data/codex-a7-quota-XXXXXX")
resolved_fixture=$(realpath -- "$fixture_root")
[[ "$resolved_fixture" == "$repo_root"/data/codex-a7-quota-* ]]

# Individual tests close their servers, unmount, and remove their exact fixtures.
# Remove the separate Vite cache only after every fixture has been cleaned.
cleanup() {
  local original_status=$?
  trap - EXIT
  cache="$resolved_fixture/slither-neuroevo-vite-cache"
  if [[ -d "$cache" && ! -L "$cache" && "$(realpath -- "$cache")" == "$cache" ]]; then
    rm -rf -- "$cache"
  fi
  if ! rmdir -- "$resolved_fixture"; then
    printf 'Quota fixture still contains files: %s\n' "$resolved_fixture" >&2
    if (( original_status == 0 )); then original_status=1; fi
  fi
  exit "$original_status"
}
trap cleanup EXIT

parent_namespace=$(readlink /proc/self/ns/mnt)
TMPDIR="$resolved_fixture" SLITHER_PRIVATE_QUOTA_TEST=1 \
  SLITHER_PARENT_MOUNT_NAMESPACE="$parent_namespace" \
  unshare --user --map-root-user --mount --propagation private \
  node node_modules/vitest/vitest.mjs run server/rustServer.archiveTransport.native.test.ts \
  --maxWorkers=1 --reporter=dot -t 'real private-filesystem exhaustion'
