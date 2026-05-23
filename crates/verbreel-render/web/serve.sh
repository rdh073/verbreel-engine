#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
PORT="${PORT:-8001}"
echo "Serving on http://localhost:$PORT"
echo "After opening, click 'Run' and let the page download wasm_frame.png."
python3 -m http.server "$PORT"
