#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

if [ "$(uname -s)" != "Darwin" ]; then
    echo "Charme.app 只能在 macOS 上构建。" >&2
    exit 1
fi

if ! command -v cargo-packager >/dev/null 2>&1; then
    echo "缺少 cargo-packager 0.11.8；请运行：" >&2
    echo "  cargo install cargo-packager --version 0.11.8 --locked" >&2
    exit 1
fi

case "$(cargo packager --version)" in
    *" 0.11.8") ;;
    *)
        echo "需要 cargo-packager 0.11.8；请安装项目固定版本。" >&2
        exit 1
        ;;
esac

cargo packager --config Packager.toml
printf '\n已构建：%s\n' "$ROOT/target/release/bundle/Charme.app"
