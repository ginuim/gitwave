#!/usr/bin/env bash
# 从仓库根目录 appicon.png 生成圆角母版 appicon-tauri.png，并执行 tauri icon。
# 依赖：ImageMagick (magick)、python3、pnpm
# 说明：若源图角部为「棋盘格假透明」（不透明像素），请勿直接对 appicon.png 跑 tauri icon，
#       须先用本脚本裁出与 iOS 图标一致的圆角透明区域，再生成各平台资源。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

command -v magick >/dev/null 2>&1 || {
  echo "需要 ImageMagick：请安装后确保 magick 在 PATH 中" >&2
  exit 1
}
command -v python3 >/dev/null 2>&1 || {
  echo "需要 python3" >&2
  exit 1
}

[[ -f appicon.png ]] || {
  echo "缺少根目录 appicon.png" >&2
  exit 1
}

W=$(magick identify -ping -format %w appicon.png)
H=$(magick identify -ping -format %h appicon.png)
[[ "$W" == "$H" ]] || {
  echo "appicon.png 必须为正方形（当前 ${W}x${H}）" >&2
  exit 1
}

R=$(python3 -c "print(max(8, int($W * 0.2237)))")

magick appicon.png \( +clone -alpha transparent -background none \
  -fill white -draw "roundrectangle 0,0 $((W - 1)),$((W - 1)),$R,$R" \) \
  -compose DstIn -composite PNG32:appicon-tauri.png

pnpm exec tauri icon appicon-tauri.png
