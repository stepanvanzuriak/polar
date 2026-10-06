#!/bin/sh
set -eu

repo="stepanvanzuriak/polar"
prefix="${POLAR_HOME:-$HOME/.polar}"

fail() {
  echo "error: $1" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

need curl
need tar

case "$(uname -s)" in
  Linux) os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) fail "unsupported OS: $(uname -s)" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

target="$arch-$os"

if [ -n "${POLAR_VERSION:-}" ]; then
  version="${POLAR_VERSION#v}"
else
  version=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" |
    sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$version" ] || fail "could not find the latest release"
fi

name="polar-$version-$target"
url="https://github.com/$repo/releases/download/v$version/$name.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading polar $version for $target"
curl -fsSL "$url" -o "$tmp/$name.tar.gz" || fail "download failed: $url"
curl -fsSL "$url.sha256" -o "$tmp/$name.tar.gz.sha256" || fail "checksum download failed"

expected=$(cut -d ' ' -f 1 "$tmp/$name.tar.gz.sha256")

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d ' ' -f 1)
else
  actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d ' ' -f 1)
fi

[ "$expected" = "$actual" ] || fail "checksum mismatch"

tar xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$prefix/bin"
cp "$tmp/$name/polar" "$prefix/bin/polar"
chmod +x "$prefix/bin/polar"

echo "Installed polar $version to $prefix/bin/polar"

case ":$PATH:" in
  *":$prefix/bin:"*) ;;
  *) echo "Add it to your PATH:  export PATH=\"$prefix/bin:\$PATH\"" ;;
esac

command -v node >/dev/null 2>&1 || echo "note: polar runs programs with Node.js, which was not found on your PATH"
