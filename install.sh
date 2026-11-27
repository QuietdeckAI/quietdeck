#!/bin/sh
# Install Quietdeck solo and the quietdeck command for this user from the latest GitHub release:
#   curl -fsSL https://quietdeck.ai/install.sh | sh
# They go into ~/.local/lib/quietdeck/<version>/, with symlinks in ~/.local/bin. The download is checked against the
# release's SHA256SUMS. Set QUIETDECK_VERSION=v0.1.0 to pick a release.
# Adapted from Fleetlayer's install.sh (Apache 2.0, see NOTICE).
set -eu
repo=QuietdeckAI/quietdeck
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *) echo "quietdeck: no release is built for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
version=${QUIETDECK_VERSION:-$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)}
[ -n "$version" ] || { echo "quietdeck: couldn't find the latest release" >&2; exit 1; }
name="quietdeck-$version-$target"
base="https://github.com/$repo/releases/download/$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $name..."
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
expected=$(grep "  $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1); fi
[ -n "$expected" ] && [ "$actual" = "$expected" ] || { echo "quietdeck: checksum mismatch for $name.tar.gz" >&2; exit 1; }
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
lib="$HOME/.local/lib/quietdeck"
mkdir -p "$lib" "$HOME/.local/bin"
rm -rf "${lib:?}/$version"
mv "$tmp/$name" "$lib/$version"
for b in quietdeck-solo quietdeck; do
  ln -sfn "$lib/$version/$b" "$HOME/.local/bin/$b"
done
echo "Installed Quietdeck $version in $lib/$version, linked from ~/.local/bin."
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Add ~/.local/bin to your PATH, then run quietdeck-solo install again after any PATH change." ;; esac
echo "Next: sign gog in to your Gmail, then run: quietdeck-solo init && quietdeck-solo install && quietdeck-solo open"
