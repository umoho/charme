#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

PROFILE=debug
ACTION=build-and-run
PROFILE_SET=0
ACTION_SET=0
PACKAGER_CONFIG=

usage() {
    cat <<EOF
用法：$(basename "$0") [选项]

默认先构建 debug 版本，再运行应用。

选项：
  --debug                 构建并运行 debug 版本（默认）
  --release               构建并运行 release 版本
  --profile <名称>        指定构建配置，可选 debug 或 release
  --run-only              仅运行，不构建
  --build-only            仅构建，不运行
  -h, --help              显示帮助
EOF
}

error() {
    echo "错误：$*" >&2
    echo "运行 \"$(basename "$0") --help\" 查看用法。" >&2
    exit 1
}

set_profile() {
    if [ "$PROFILE_SET" -eq 1 ]; then
        error "不能重复指定构建配置。"
    fi
    PROFILE=$1
    case "$PROFILE" in
        debug|release) ;;
        *) error "不支持的构建配置：$PROFILE（只能是 debug 或 release）。" ;;
    esac
    PROFILE_SET=1
}

set_action() {
    if [ "$ACTION_SET" -eq 1 ]; then
        error "不能同时指定 --run-only 和 --build-only。"
    fi
    ACTION=$1
    ACTION_SET=1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --debug|-d)
            set_profile debug
            ;;
        --release|-r)
            set_profile release
            ;;
        --profile)
            [ "$#" -ge 2 ] || error "--profile 需要一个值。"
            set_profile "$2"
            shift
            ;;
        --profile=*)
            set_profile "${1#*=}"
            ;;
        --run-only|--only-run|--run)
            set_action run-only
            ;;
        --build-only|--only-build|--build)
            set_action build-only
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            error "未知参数：$1"
            ;;
    esac
    shift
done

if [ "$(uname -s)" != "Darwin" ]; then
    echo "Charme.app 只能在 macOS 上构建或运行。" >&2
    exit 1
fi

APP="$ROOT/target/$PROFILE/bundle/Charme.app"

cleanup() {
    if [ -n "$PACKAGER_CONFIG" ]; then
        rm -f "$PACKAGER_CONFIG"
    fi
}
trap cleanup EXIT

build_app() {
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

    # Packager.toml currently describes the release layout. Generate a temporary
    # profile-specific copy so cargo-packager also picks up debug binaries and
    # writes the bundle to target/debug when no profile was requested.
    PACKAGER_CONFIG=$(mktemp "$ROOT/.Packager.XXXXXX")
    mv "$PACKAGER_CONFIG" "$PACKAGER_CONFIG.toml"
    PACKAGER_CONFIG="$PACKAGER_CONFIG.toml"
    if [ "$PROFILE" = "release" ]; then
        cp "$ROOT/Packager.toml" "$PACKAGER_CONFIG"
    else
        sed \
            -e 's#target/release#target/debug#g' \
            -e 's#cargo build --release#cargo build#g' \
            "$ROOT/Packager.toml" > "$PACKAGER_CONFIG"
    fi

    cargo packager --config "$PACKAGER_CONFIG"
    printf '\n已构建（%s）：%s\n' "$PROFILE" "$APP"
}

if [ "$ACTION" != "run-only" ]; then
    build_app
fi

if [ "$ACTION" != "build-only" ]; then
    if [ ! -d "$APP" ]; then
        echo "未找到 $APP；请先构建对应的 $PROFILE 版本，或去掉 --run-only。" >&2
        exit 1
    fi
    open "$APP"
    printf '已运行（%s）：%s\n' "$PROFILE" "$APP"
fi
