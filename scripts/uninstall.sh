#!/bin/sh
# stele uninstaller. Removes the single installed `stele` binary. There is no data
# dir, symlink tree, or running server to tear down (grove's model scaled to one
# executable) — install.sh drops exactly one file, so uninstall removes exactly that.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/uninstall.sh | sh
#   STELE_INSTALL_DIR=/custom sh uninstall.sh
#
# Environment:
#   STELE_INSTALL_DIR   install dir (default: ~/.local/bin)
set -eu

install_dir="${STELE_INSTALL_DIR:-$HOME/.local/bin}"
bin="$install_dir/stele"

if [ -e "$bin" ]; then
  rm -f "$bin"
  echo "stele: removed $bin"
else
  echo "stele: nothing to remove ($bin not found)"
fi
echo "stele: done."
