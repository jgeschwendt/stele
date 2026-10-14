#!/bin/sh
# Render every Claude Design artboard under docs/figures/project/ to its PNG at 2×,
# the same pixels the canvas exports — agent-browser, headless. Run from anywhere: mise run figures.
set -e
# An isolated session: the default one is shared, and a concurrent user resizes it mid-shot.
export AGENT_BROWSER_SESSION="${AGENT_BROWSER_SESSION:-stele-figures}"
P=$(cd "$(dirname "$0")" && pwd)
S=$P/project
shoot() { # <board .dc.html> <out.png>
  h=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["boards"][sys.argv[2]]["h"])' "$S/canvas.json" "$1")
  agent-browser set viewport 1600 "$h" 2 >/dev/null
  agent-browser open "file://$S/$1" >/dev/null
  agent-browser wait 1500 >/dev/null
  agent-browser screenshot "$P/$2" >/dev/null
  printf '%s %s\n' "$2" "$(file "$P/$2" | sed 's/.*PNG image data, //; s/,.*//')"
}
shoot AuthoredVsExtracted.dc.html authored-vs-extracted.png
shoot DegradationLadder.dc.html   degradation-ladder.png
agent-browser close >/dev/null
