#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
APP="$ROOT/target/release/bundle/Charme.app"

if [ ! -d "$APP" ]; then
    echo "未找到 $APP；请先运行 scripts/build-macos-app.sh。" >&2
    exit 1
fi

open "$APP"
