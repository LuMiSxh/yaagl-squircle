#!/bin/sh
# Installs the latest yaagl-squircle release into ~/.local/bin (override: YAAGL_SQUIRCLE_INSTALL_DIR).
set -eu

REPO="LuMiSxh/yaagl-squircle"
DIR="${YAAGL_SQUIRCLE_INSTALL_DIR:-$HOME/.local/bin}"

[ "$(uname -s)" = "Darwin" ] || { echo "yaagl-squircle only runs on macOS" >&2; exit 1; }
case "$(uname -m)" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

name="yaagl-squircle-$target.tar.gz"
url="https://github.com/$REPO/releases/latest/download"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$url/$name" -o "$tmp/$name"
curl -fsSL "$url/$name.sha256" -o "$tmp/$name.sha256"
(cd "$tmp" && shasum -a 256 -c "$name.sha256")
tar -xzf "$tmp/$name" -C "$tmp"

mkdir -p "$DIR"
install -m 755 "$tmp/yaagl-squircle" "$DIR/yaagl-squircle"
echo "installed $DIR/yaagl-squircle"

case ":$PATH:" in
  *":$DIR:"*) ;;
  *)
    rc="$HOME/.zshrc"
    printf '\nexport PATH="%s:$PATH"\n' "$DIR" >> "$rc"
    echo "added $DIR to PATH in $rc; open a new terminal"
    ;;
esac
