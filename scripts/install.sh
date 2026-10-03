#!/bin/sh
set -eu

repo=https://github.com/rustiqz/monitra
dest=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir)
      [ "$#" -ge 2 ] || { echo 'install: --dir needs a path' >&2; exit 2; }
      [ -n "$2" ] || { echo 'install: --dir needs a non-empty path' >&2; exit 2; }
      dest=$2
      shift 2
      ;;
    -h|--help)
      echo 'Usage: install.sh [--dir INSTALL_DIR]'
      exit 0
      ;;
    *) echo "install: unknown option: $1" >&2; exit 2 ;;
  esac
done
[ -n "$dest" ] || dest=${HOME:?HOME must be set}/.local/bin

os=$(uname -s)
arch=$(uname -m)
if [ "$os" != Linux ]; then
  echo "install: unsupported OS $os (Linux only)" >&2
  exit 1
fi
case "$arch" in
  x86_64|amd64) target=x86_64-unknown-linux-musl ;;
  aarch64|arm64) target=aarch64-unknown-linux-musl ;;
  *) echo "install: unsupported Linux architecture $arch (supported: x86_64, aarch64)" >&2; exit 1 ;;
esac

for command in curl sha256sum tar install mktemp; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "install: required command missing: $command" >&2
    exit 1
  }
done

latest=$(curl --fail --silent --show-error --location --output /dev/null \
  --write-out '%{url_effective}' "$repo/releases/latest")
version=${latest##*/}
case "$version" in
  v[0-9]* ) ;;
  *) echo "install: could not determine latest release from $latest" >&2; exit 1 ;;
esac
archive="monitra-${version}-${target}.tar.gz"
base="$repo/releases/download/$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl --fail --silent --show-error --location --output "$tmp/SHA256SUMS" "$base/SHA256SUMS"
curl --fail --silent --show-error --location --output "$tmp/$archive" "$base/$archive"

expected=$(awk -v file="$archive" '$2 == file { print $1 }' "$tmp/SHA256SUMS")
if [ -z "$expected" ]; then
  echo "install: $archive is absent from SHA256SUMS" >&2
  exit 1
fi
printf '%s  %s\n' "$expected" "$tmp/$archive" | sha256sum -c -
tar -xOzf "$tmp/$archive" "${archive%.tar.gz}/monitra" > "$tmp/monitra"
mkdir -p "$dest"
install -m 755 "$tmp/monitra" "$dest/monitra"
echo "Installed $version to $dest/monitra"
"$dest/monitra" --version
