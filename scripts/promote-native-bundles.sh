#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <x64-artifact-directory> <arm64-artifact-directory>" >&2
  exit 64
fi

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
resolve_artifact_directory() {
  local root=$1
  local architecture=$2
  local binary="local-cache-linux-${architecture}"
  local candidate=''
  local count=0
  while IFS= read -r directory; do
    candidate=$directory
    ((count += 1))
  done < <(find "$root" -type f -name "$binary" -exec dirname {} \; | sort -u)
  [[ $count -eq 1 ]] || {
    echo "expected exactly one ${binary} below ${root}" >&2
    exit 1
  }
  printf '%s\n' "$candidate"
}

x64_artifact=$(resolve_artifact_directory "$1" x64)
arm64_artifact=$(resolve_artifact_directory "$2" arm64)

verify_artifact() {
  local directory=$1
  local architecture=$2
  local binary="local-cache-linux-${architecture}"
  local checksum="${binary}.sha256"

  [[ -f "${directory}/${binary}" && ! -L "${directory}/${binary}" ]] || {
    echo "missing regular artifact binary: ${directory}/${binary}" >&2
    exit 1
  }
  [[ -f "${directory}/${checksum}" && ! -L "${directory}/${checksum}" ]] || {
    echo "missing regular artifact checksum: ${directory}/${checksum}" >&2
    exit 1
  }
  [[ $(find "$directory" -maxdepth 1 -type f | wc -l | tr -d ' ') -eq 2 ]] || {
    echo "artifact directory must contain only the binary and checksum" >&2
    exit 1
  }
  (cd "$directory" && sha256sum --strict --check "$checksum")
}

verify_artifact "$x64_artifact" x64
verify_artifact "$arm64_artifact" arm64

install -m 0755 "$x64_artifact/local-cache-linux-x64" "$repository_root/restore/dist/local-cache-linux-x64"
install -m 0755 "$x64_artifact/local-cache-linux-x64" "$repository_root/save/dist/local-cache-linux-x64"
install -m 0755 "$arm64_artifact/local-cache-linux-arm64" "$repository_root/restore/dist/local-cache-linux-arm64"
install -m 0755 "$arm64_artifact/local-cache-linux-arm64" "$repository_root/save/dist/local-cache-linux-arm64"

(
  cd "$repository_root"
  sha256sum \
    restore/dist/local-cache-linux-x64 \
    restore/dist/local-cache-linux-arm64 \
    save/dist/local-cache-linux-x64 \
    save/dist/local-cache-linux-arm64 \
    > checksums.sha256
)

echo "Promoted verified x64 and arm64 bundles; review the binary diff and checksums before committing."
