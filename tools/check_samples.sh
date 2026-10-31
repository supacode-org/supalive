#!/usr/bin/env bash
# 開發用驗證腳本：用本機 ffmpeg 產生 H.264 / HEVC 測試串流，再用 supalive 轉檔並檢查能否完整解碼。
# ffmpeg 只是開發者機器上的檢查工具，不會進入發佈包。
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q
B=target/release/supalive
T=$(mktemp -d)
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i sine=frequency=440 \
  -t 150 -c:v libx264 -bf 2 -g 60 -pix_fmt yuv420p -c:a aac "$T/h264.flv"
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=1080x1920:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 \
  -t 20 -c:v libx265 -x265-params log-level=error -c:a aac "$T/hevc.flv"
python3 tools/to_codec12.py "$T/hevc.flv" "$T/hevc12.flv"
$B remux "$T/h264.flv" -o "$T/out" -n h264 --segment 1
$B remux "$T/hevc12.flv" -o "$T/out" -n hevc12
find "$T/out" -name '*.mp4' | sort | while read -r f; do
  ffmpeg -nostdin -v error -xerror -i "$f" -f null - && echo "OK  $f $(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f")s"
done
echo "輸出在 $T/out"
