#!/bin/bash
# stele installer. Resolves a release on jgeschwendt/stele, downloads the single
# per-platform tarball (one static `stele` binary) + its checksum sidecar, verifies
# the sha256, and installs the binary to ${STELE_INSTALL_DIR:-$HOME/.local/bin}. No
# self-update machinery and no bundle — the artifact is one executable, so install is
# download → verify → drop the binary on PATH.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | channel=canary bash   # prereleases
#   curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash -s v0.1.0
#
# Environment:
#   channel             release channel: stable (default), canary
#   STELE_INSTALL_DIR   install dir (default: ~/.local/bin)
#   STELE_ARTIFACT_DIR  install from a LOCAL artifact dir (skips the GitHub fetch);
#                       the dir holds stele-<target>.tar.gz + .sha256 (tests/install_smoke.sh)
set -euo pipefail

repo="jgeschwendt/stele"
install_dir="${STELE_INSTALL_DIR:-$HOME/.local/bin}"
channel="${channel:-stable}"
version="${1:-}"

die() { echo "stele: $1" >&2; exit 1; }

# Accept-Encoding: identity defeats corp MITM proxies that recompress gzip mid-flight
# (re-compression changes bytes → sha256 mismatch).
fetch() { curl -fsSL -H "Accept-Encoding: identity" "$1" -o "$2"; }
fetch_text() { curl -fsSL -H "Accept-Encoding: identity" "$1"; }

# sha256 of a file as a bare hash — sha256sum on linux, shasum -a 256 on darwin.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# Platform string <arch>-<os>, matching scripts/release.sh + the release matrix.
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac
case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) die "unsupported OS: $(uname -s)" ;;
esac
target="${arch}-${os}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tarball="$tmp/stele-$target.tar.gz"
sidecar="$tarball.sha256"

if [ -n "${STELE_ARTIFACT_DIR:-}" ]; then
  # Local artifacts (no network) — the hermetic smoke-test path. The dir is a single
  # version's output from scripts/release.sh: stele-<target>.tar.gz + .sha256.
  src="$STELE_ARTIFACT_DIR/stele-$target.tar.gz"
  [ -f "$src" ] || die "local artifact not found: $src (STELE_ARTIFACT_DIR)"
  [ -f "$src.sha256" ] || die "local checksum not found: $src.sha256"
  cp "$src" "$tarball"
  cp "$src.sha256" "$sidecar"
  echo "stele: installing ${target} from ${STELE_ARTIFACT_DIR}"
else
  # Resolve the release tag for the channel (unless a version was pinned). canary is a
  # prerelease tag suffix (v<semver>-canary.N); stable is the latest release.
  if [ -z "$version" ]; then
    if [ "$channel" = "stable" ]; then
      tag=$(curl -fsSI -H "Accept-Encoding: identity" \
        "https://github.com/${repo}/releases/latest" |
        grep -i '^location:' | sed -E 's|.*/tag/([^[:space:]]+).*|\1|')
      [ -n "$tag" ] || die "could not resolve the latest stable release"
    else
      tag=$(fetch_text "https://api.github.com/repos/${repo}/releases" |
        grep -oE '"tag_name": "v[^"]+-'"${channel}"'\.[0-9]+"' |
        sed -E 's/.*"(v[^"]+)".*/\1/' |
        sort -t. -k1,1V -k2,2V -k3,3V -k4,4n | tail -1)
      [ -n "$tag" ] || die "no ${channel} releases found on ${repo}"
    fi
  else
    # A pinned arg may be given with or without the leading v.
    case "$version" in v*) tag="$version" ;; *) tag="v$version" ;; esac
  fi
  vsn="${tag#v}"
  base="https://github.com/${repo}/releases/download/${tag}"
  echo "stele: installing ${vsn} (${target}) from ${repo}"

  fetch "$base/stele-$target.tar.gz" "$tarball" \
    || die "release asset unreachable: stele-$target.tar.gz (no build for $target on $tag?)"
  fetch "$base/stele-$target.tar.gz.sha256" "$sidecar" \
    || die "release asset unreachable: stele-$target.tar.gz.sha256"
fi

# Verify the checksum sidecar (bare hash written by scripts/release.sh) before install.
want=$(cat "$sidecar")
got=$(sha256_of "$tarball")
[ "$want" = "$got" ] || die "checksum mismatch for stele-$target.tar.gz (want $want, got $got)"

tar -xzf "$tarball" -C "$tmp"
[ -f "$tmp/stele" ] || die "tarball did not contain a stele binary"

mkdir -p "$install_dir"
install -m 755 "$tmp/stele" "$install_dir/stele"

# Warn (don't fail) if the install dir is not on PATH — the binary is installed either way.
case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) echo "stele: ${install_dir} is not on your PATH — add it to use \`stele\` directly" ;;
esac

echo "stele: installed → ${install_dir}/stele"
"$install_dir/stele" --version
