#!/usr/bin/env sh
# Installs the latest (or a pinned) `paws` release binary for the current
# platform to ~/.local/bin. Mirrors `actions/paws-up`'s own install logic
# (the CI equivalent) - kept here as a single reusable, non-CI script so a
# repo's local dev setup (e.g. `mbround18/helm-charts`' `make install-paws`)
# doesn't have to duplicate the OS/arch-detection + download logic itself.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/mbround18/paws/main/scripts/install.sh | sh
# Pin a version:
#   PAWS_VERSION=v0.0.1-prerelease.18 curl -fsSL .../install.sh | sh
set -eu

os=""
case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *)
    echo "Unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

arch=""
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *)
    echo "Unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

case "$os" in
  linux)
    # Auto-detect musl vs gnu rather than assuming - a plain uname can't
    # tell you this, but the presence of musl's loader can.
    libc=gnu
    if ls /lib/ld-musl-*.so.1 >/dev/null 2>&1; then
      libc=musl
    fi
    target="${arch}-unknown-linux-${libc}"
    exe_name="paws"
    ;;
  macos)
    target="${arch}-apple-darwin"
    exe_name="paws"
    ;;
  windows)
    target="x86_64-pc-windows-gnu"
    exe_name="paws.exe"
    ;;
esac
echo "Resolved target: $target"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

version="${PAWS_VERSION:-}"
if [ -z "$version" ]; then
  # `/releases/latest` skips prereleases, and paws is pre-1.0 (every
  # release so far is one) - list and take the newest instead, same as
  # `actions/paws-up`'s `gh release list --limit 1` does in CI. Fetched to
  # a temp file first, not piped straight into `grep -m1` - grep exiting
  # after its first match closes the pipe while curl is still writing the
  # rest of the (much larger) response body, which curl reports as a
  # spurious "Failure writing output to destination" even though the
  # version was already resolved correctly by then.
  curl -fsSL https://api.github.com/repos/mbround18/paws/releases -o "$tmp_dir/releases.json"
  version="$(grep -m1 '"tag_name"' "$tmp_dir/releases.json" | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  if [ -z "$version" ]; then
    echo "Could not resolve the latest paws release" >&2
    exit 1
  fi
fi
echo "Resolved version: $version"

version_no_v="${version#v}"
archive="paws-${version_no_v}-${target}.zip"
url="https://github.com/mbround18/paws/releases/download/${version}/${archive}"
echo "Downloading: $url"

install_dir="${PAWS_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"

curl -fsSL -o "$tmp_dir/$archive" "$url"
unzip -o -j "$tmp_dir/$archive" -d "$install_dir" >/dev/null
chmod +x "$install_dir/$exe_name"

echo "Installed $exe_name to $install_dir"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *)
    echo "Note: $install_dir is not on your PATH - add it, e.g.:"
    echo "  export PATH=\"$install_dir:\$PATH\""
    ;;
esac

# Sanity check before trusting this install - same "verify it actually
# runs, not just that the download succeeded" standard the release
# pipeline itself holds every binary to.
"$install_dir/$exe_name" --version
