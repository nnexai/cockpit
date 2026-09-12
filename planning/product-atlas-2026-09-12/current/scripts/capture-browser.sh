#!/usr/bin/env bash
set -Eeuo pipefail

# Run with the generated, paired production extension bundle from the receipt:
#   COCKPIT_EXTENSION_DIR=/tmp/cab12/cockpit-state/browser/extensions/... \
#     ./capture-browser.sh

ROOT="${COCKPIT_BROWSER_RUN_ROOT:-/tmp/cab12-rerun}"
PROFILE="$ROOT/profileatlas-browser"
FIXTURE="${COCKPIT_FIXTURE_DIR:-$PWD/poc/interactive-browser-panel/fixture}"
CDP_PORT="${COCKPIT_CDP_PORT:-4206}"
FIXTURE_PORT="${COCKPIT_FIXTURE_PORT:-4205}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

: "${COCKPIT_EXTENSION_DIR:?set COCKPIT_EXTENSION_DIR to the generated production bundle}"
mkdir -p "$ROOT" "$PROFILE"

cleanup() {
  test -z "${CHROME_PID:-}" || kill "$CHROME_PID" 2>/dev/null || true
  test -z "${FIXTURE_PID:-}" || kill "$FIXTURE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

python3 -m http.server "$FIXTURE_PORT" --bind 127.0.0.1 --directory "$FIXTURE" >/dev/null 2>&1 &
FIXTURE_PID=$!
/opt/google/chrome/chrome --headless=new --no-first-run --no-default-browser-check \
  --user-data-dir="$PROFILE" --remote-debugging-address=127.0.0.1 --remote-debugging-port="$CDP_PORT" \
  --noerrdialogs --ozone-platform=headless --ozone-override-screen-size=1440,900 \
  --use-angle=swiftshader about:blank >/dev/null 2>&1 &
CHROME_PID=$!

for _ in {1..50}; do curl -fsS "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null && break; sleep 0.1; done
COCKPIT_CDP_URL="http://127.0.0.1:$CDP_PORT" COCKPIT_FIXTURE_URL="http://127.0.0.1:$FIXTURE_PORT/" \
  timeout --signal=TERM 25s node "$SCRIPT_DIR/capture-browser.mjs"
