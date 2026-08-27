#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
user_home_dir=${HOME:?HOME must be set}
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/wechat-mp-mcp-build.XXXXXX")
dist_dir="$project_dir/dist"

trap 'rm -rf "$build_dir"' EXIT

RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${user_home_dir}=/home/user --remap-path-prefix=${project_dir}=."
export RUSTFLAGS

cargo build \
    --quiet \
    --release \
    --manifest-path "$project_dir/Cargo.toml" \
    --target-dir "$build_dir"

mkdir -p "$dist_dir"
cp "$build_dir/release/wechat-mp-mcp" "$dist_dir/wechat-mp-mcp"
chmod 755 "$dist_dir/wechat-mp-mcp"

if strings "$dist_dir/wechat-mp-mcp" | LC_ALL=C grep -F "$user_home_dir" >/dev/null; then
    echo "error: release binary still contains the local home path" >&2
    exit 1
fi

echo "anonymous release: dist/wechat-mp-mcp"
