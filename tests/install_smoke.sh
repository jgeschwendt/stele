#!/bin/sh
# Hermetic smoke test for the distribution scripts (scripts/{release,install,uninstall}.sh):
# assemble a local artifact (no network), install it FROM that local dir via
# STELE_ARTIFACT_DIR, assert the binary lands and `--version` runs, then uninstall and
# assert it is gone. No GitHub, no network — safe to run inside `cargo test` (tests/install.rs).
#
# Usage: sh tests/install_smoke.sh
set -eu

# shellcheck disable=SC1007  # `CDPATH= cd` is an env-prefix (neutralize CDPATH for the cd), not an assignment
ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd) # repo root
cd "$ROOT"

# Compute the target the same way release.sh + install.sh do.
arch=$(uname -m); os=$(uname -s)
case "$arch" in arm64 | aarch64) arch=aarch64 ;; x86_64 | amd64) arch=x86_64 ;; esac
case "$os" in Darwin) os=darwin ;; Linux) os=linux ;; esac
TARGET="${arch}-${os}"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/stele-smoke.XXXXXX") # macOS mktemp -d ignores TMPDIR
trap 'rm -rf "$WORK"' EXIT
DIST="$WORK/dist"
BIN_DIR="$WORK/bin"
HOME_DIR="$WORK/home"
mkdir -p "$BIN_DIR" "$HOME_DIR"

echo "smoke: assembling local artifact ($TARGET)"
DIST="$DIST" sh "$ROOT/scripts/release.sh" >/dev/null
VERSION=$(cat "$DIST/latest")
ART="$DIST/$VERSION"
[ -f "$ART/stele-$TARGET.tar.gz" ] || { echo "FAIL: release.sh produced no tarball at $ART"; exit 1; }
[ -f "$ART/stele-$TARGET.tar.gz.sha256" ] || { echo "FAIL: release.sh produced no sha256 sidecar"; exit 1; }

echo "smoke: installing from local artifact dir (no network)"
HOME="$HOME_DIR" STELE_INSTALL_DIR="$BIN_DIR" STELE_ARTIFACT_DIR="$ART" \
  bash "$ROOT/scripts/install.sh"

echo "smoke: a download cut past the shebang line is a syntax error, never a partial run"
# Every line end after the shebang (preamble included) + every byte of the last line
# short of its newline (dropping only that leaves the whole script, which runs).
for CUT in $(LC_ALL=C awk 'NR > 2 { print n } { n += l = length($0) + 1 } END { for (i = n - l + 1; i < n - 1; i++) print i }' "$ROOT/scripts/install.sh"); do
  if ERR=$(head -c "$CUT" "$ROOT/scripts/install.sh" | HOME="$HOME_DIR" STELE_INSTALL_DIR="$WORK/cut-bin" STELE_ARTIFACT_DIR="$ART" bash -s v0.0.1 2>&1); then
    echo "FAIL: install.sh cut at byte $CUT exited 0"; exit 1
  fi
  case "$ERR" in
    *"unexpected end of file"*) : ;;
    *) echo "FAIL: install.sh cut at byte $CUT ran instead of failing to parse: $ERR"; exit 1 ;;
  esac
done

echo "smoke: asserting the binary landed and runs"
[ -x "$BIN_DIR/stele" ] || { echo "FAIL: stele not installed at $BIN_DIR/stele"; exit 1; }
GOT=$("$BIN_DIR/stele" --version)
case "$GOT" in
  "stele $VERSION") : ;;
  *) echo "FAIL: --version was '$GOT', want 'stele $VERSION'"; exit 1 ;;
esac

echo "smoke: uninstalling"
HOME="$HOME_DIR" STELE_INSTALL_DIR="$BIN_DIR" sh "$ROOT/scripts/uninstall.sh"
[ ! -e "$BIN_DIR/stele" ] || { echo "FAIL: uninstall left $BIN_DIR/stele behind"; exit 1; }
ln -s "$WORK/gone" "$BIN_DIR/stele" # a hand-made link whose target was deleted
STELE_INSTALL_DIR="$BIN_DIR" sh "$ROOT/scripts/uninstall.sh" >/dev/null
[ ! -L "$BIN_DIR/stele" ] || { echo "FAIL: uninstall left a dangling $BIN_DIR/stele"; exit 1; }

echo "smoke: PASS — release → install → --version ($GOT) → uninstall"
