#!/bin/sh
# Installs the bb binary into ~/.local/bin. No sudo, no Rust toolchain.
set -eu

REPO="biokraft/bbcloud"
INSTALL_DIR="${BB_INSTALL_DIR:-$HOME/.local/bin}"

fail() { printf 'error: %s\n' "$1" >&2; exit 1; }

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)        triple="aarch64-apple-darwin" ;;
  Darwin/x86_64)       triple="x86_64-apple-darwin" ;;
  Linux/x86_64)        triple="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64|Linux/arm64) triple="aarch64-unknown-linux-gnu" ;;
  *) fail "unsupported platform $os/$arch — supported: macOS arm64/x86_64, Linux x86_64/aarch64" ;;
esac

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar  >/dev/null 2>&1 || fail "tar is required"

tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n 1)"
[ -n "$tag" ] || fail "cannot determine the latest release tag"

asset_base="bbcloud-$tag-$triple"
archive="$asset_base.tar.gz"
base="https://github.com/$REPO/releases/download/$tag"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

printf 'downloading %s\n' "$archive"
curl -fsSL -o "$tmp/$archive" "$base/$archive"
curl -fsSL -o "$tmp/$asset_base.sha256" "$base/$asset_base.sha256"

expected="$(awk '{print $1; exit}' "$tmp/$asset_base.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$archive" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')"
else
  fail "neither sha256sum nor shasum is available to verify the download"
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch — refusing to install"

tar -xzOf "$tmp/$archive" bb > "$tmp/bb"
if [ ! -f "$tmp/bb" ] || [ -L "$tmp/bb" ] || [ ! -s "$tmp/bb" ]; then
  fail "archive does not contain a regular-file bb binary"
fi

mkdir -p "$INSTALL_DIR"
mv "$tmp/bb" "$INSTALL_DIR/bb"
chmod 755 "$INSTALL_DIR/bb"

printf 'installed bb %s to %s\n' "$tag" "$INSTALL_DIR"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) printf '\nwarning: %s is not on your PATH. Add it:\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac
