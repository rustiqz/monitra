#!/bin/sh
set -eu

artifacts=${1:?usage: prepare-image-context.sh ARTIFACT_DIR VERSION}
version=${2:?usage: prepare-image-context.sh ARTIFACT_DIR VERSION}
case "$version" in
  *[!0-9.]*|'' ) echo "image: invalid version: $version" >&2; exit 1 ;;
esac

for mapping in amd64:x86_64 arm64:aarch64; do
  arch=${mapping%%:*}
  rust_arch=${mapping#*:}
  archive="monitra-v${version}-${rust_arch}-unknown-linux-musl.tar.gz"
  expected=$(awk -v file="$archive" '$2 == file { print $1 }' "$artifacts/SHA256SUMS")
  if [ -z "$expected" ]; then
    echo "image: checksum missing for $archive" >&2
    exit 1
  fi
  printf '%s  %s\n' "$expected" "$artifacts/$archive" | sha256sum -c -
  mkdir -p "docker/release/$arch"
  tar -xOzf "$artifacts/$archive" "${archive%.tar.gz}/monitra" > "docker/release/$arch/monitra"
  chmod 755 "docker/release/$arch/monitra"
done
