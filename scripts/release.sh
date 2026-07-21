#!/bin/sh
# Assemble a stele release ARTIFACT locally — the same tarball + checksum the CI
# matrix (.github/workflows/release.yml) publishes, produced on disk so the artifact
# can be built and smoke-tested (tests/install_smoke.sh) without CI or network.
#
# Under $DIST/<version>/:
#   stele-<target>.tar.gz         the single stripped `stele` binary (release build)
#   stele-<target>.tar.gz.sha256  bare-hash checksum sidecar scripts/install.sh verifies
# and $DIST/latest names the version.
#
# There is no ERTS bundle and no bootstrap CLI (grove's model): stele ships one static
# executable, so the artifact is exactly that one binary in a tarball.
#
# Consumed by scripts/install.sh (STELE_ARTIFACT_DIR) + tests/install_smoke.sh.
set -eu

# shellcheck disable=SC1007  # `CDPATH= cd` is an env-prefix (neutralize CDPATH for the cd), not an assignment
cd "$(CDPATH= cd "$(dirname "$0")/.." && pwd)" # repo root
DIST="${DIST:-$PWD/dist}"

# Target = <arch>-<os>, matching install.sh's detection and the release matrix.
arch=$(uname -m)
case "$arch" in arm64 | aarch64) arch=aarch64 ;; x86_64 | amd64) arch=x86_64 ;; esac
os=$(uname -s)
case "$os" in
Darwin) os=darwin ;;
Linux) os=linux ;;
*)
  echo "release: unsupported OS: $os" >&2
  exit 1
  ;;
esac
TARGET="${arch}-${os}"

# Version: the [package] version in Cargo.toml is the single source `stele --version`
# also reports. CI stamps it from the tag before calling this script (release.yml).
VERSION=$(grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

echo "release: building stele ($VERSION, $TARGET)"
cargo build --release --bin stele

# Stage the one binary and tar it at the archive root (extract → ./stele). The
# release profile strips symbols (Cargo.toml [profile.release]).
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp target/release/stele "$STAGE/stele"
chmod +x "$STAGE/stele"

OUT="$DIST/$VERSION"
mkdir -p "$OUT"
tar -czf "$OUT/stele-$TARGET.tar.gz" -C "$STAGE" stele
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUT" && sha256sum "stele-$TARGET.tar.gz" | awk '{print $1}' >"stele-$TARGET.tar.gz.sha256")
else
  (cd "$OUT" && shasum -a 256 "stele-$TARGET.tar.gz" | awk '{print $1}' >"stele-$TARGET.tar.gz.sha256")
fi
echo "$VERSION" >"$DIST/latest"

echo "release: artifact → $OUT/stele-$TARGET.tar.gz"
echo "release: sha256   → $OUT/stele-$TARGET.tar.gz.sha256"
echo "release: channel  → $DIST/latest ($VERSION)"
