#!/usr/bin/env python3
"""抖音直播錄製：原始位元流不重新編碼，按時長分片。

抖音 CDN 的 FLV 用非標準 codec id 12 封裝 HEVC，官方 FFmpeg 無法解析。
本腳本只改寫 FLV 標籤頭（id 12 -> Enhanced FLV 'hvc1'），畫面/聲音資料原樣保留，
再交給 ffmpeg -c copy 做分片封裝。

用法：
  python3 douyin_rec.py URL [-o 目錄] [--segment 秒] [--keep N] [--format mp4|mkv|ts]
"""
import argparse
import os
import signal
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime

UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36")

stop = False


def log(msg):
    print(f"[{datetime.now():%H:%M:%S}] {msg}", flush=True)


def read_exact(r, n):
    buf = b""
    while len(buf) < n:
        chunk = r.read(n - len(buf))
        if not chunk:
            raise EOFError
        buf += chunk
    return buf


def convert_video(data):
    """codec id 12 (HEVC 擴展) -> Enhanced FLV；其他格式原樣返回。"""
    b0 = data[0]
    if b0 & 0x80 or (b0 & 0x0F) != 12 or len(data) < 5:
        return data
    frame_type = (b0 >> 4) & 0x07
    pkt_type = data[1]
    cts = data[2:5]
    payload = data[5:]
    if pkt_type == 0:    # 序列頭 (HEVCDecoderConfigurationRecord)
        return bytes([0x80 | frame_type << 4 | 0]) + b"hvc1" + payload
    if pkt_type == 1:    # 編碼幀，保留 composition time
        return bytes([0x80 | frame_type << 4 | 1]) + b"hvc1" + cts + payload
    return bytes([0x80 | frame_type << 4 | 2]) + b"hvc1"   # 序列結束


def start_ffmpeg(outdir, args):
    ext = args.format
    pattern = os.path.join(outdir, f"%Y%m%d_%H%M%S.{ext}")
    cmd = ["ffmpeg", "-hide_banner", "-loglevel", "warning",
           "-f", "flv", "-i", "pipe:0",
           "-map", "0", "-c", "copy",
           "-f", "segment", "-segment_time", str(args.segment),
           "-reset_timestamps", "1", "-strftime", "1"]
    if ext == "mp4":
        # hvc1 標記讓 QuickTime 可播；分段式 MP4 在異常中斷時也不會損壞
        cmd += ["-tag:v", "hvc1", "-segment_format", "mp4",
                "-segment_format_options", "movflags=+frag_keyframe+empty_moov+default_base_moof"]
    elif ext == "mkv":
        cmd += ["-segment_format", "matroska"]
    else:
        cmd += ["-segment_format", "mpegts"]
    if args.keep:
        # 寫入分片清單，供輪替刪除用
        cmd += ["-segment_list", os.path.join(outdir, ".segments.csv"),
                "-segment_list_type", "csv"]
    cmd.append(pattern)
    return subprocess.Popen(cmd, stdin=subprocess.PIPE)


def prune(outdir, args):
    if not args.keep:
        return
    files = sorted(f for f in os.listdir(outdir) if f.endswith("." + args.format))
    for f in files[:-args.keep]:
        os.remove(os.path.join(outdir, f))
        log(f"已刪除舊分片 {f}")


def record_once(url, outdir, args):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Referer": ""})
    resp = urllib.request.urlopen(req, timeout=15)
    if read_exact(resp, 3) != b"FLV":
        raise ValueError("回應不是 FLV")
    header = b"FLV" + read_exact(resp, 6)
    read_exact(resp, 4)  # PreviousTagSize0

    ff = start_ffmpeg(outdir, args)
    out = ff.stdin
    out.write(header + b"\x00\x00\x00\x00")
    total = 0
    last_report = time.time()
    last_prune = time.time()
    try:
        while not stop:
            th = read_exact(resp, 11)
            tag_type = th[0] & 0x1F
            size = int.from_bytes(th[1:4], "big")
            data = read_exact(resp, size)
            read_exact(resp, 4)
            if tag_type == 18:   # onMetaData 內 videocodecid=12 會誤導 ffmpeg，丟棄
                continue
            if tag_type == 9:
                data = convert_video(data)
            new_th = bytes([th[0]]) + len(data).to_bytes(3, "big") + th[4:]
            out.write(new_th + data + struct.pack(">I", 11 + len(data)))
            total += size
            now = time.time()
            if now - last_report > 60:
                log(f"錄製中，本次連線已接收 {total / 1e6:.1f} MB")
                last_report = now
            if now - last_prune > 30:
                prune(outdir, args)
                last_prune = now
    finally:
        try:
            out.close()
        except BrokenPipeError:
            pass
        ff.wait()
        resp.close()
        prune(outdir, args)


def main():
    global stop
    p = argparse.ArgumentParser()
    p.add_argument("url")
    p.add_argument("-o", "--outdir", default=os.path.expanduser("~/Movies/抖音錄製"))
    p.add_argument("--segment", type=int, default=1800, help="每個分片秒數（預設 1800 = 30 分鐘）")
    p.add_argument("--keep", type=int, default=0, help="只保留最新 N 個分片（0 = 全部保留）")
    p.add_argument("--format", choices=["mp4", "mkv", "ts"], default="mp4")
    p.add_argument("--retries", type=int, default=10, help="斷線後連續重試次數，用完視為直播結束")
    args = p.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    def on_signal(*_):
        global stop
        stop = True
        log("收到停止訊號，正在封裝最後一個分片…")
    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)

    log(f"輸出目錄：{args.outdir}")
    fails = 0
    while not stop:
        started = time.time()
        try:
            log("連線中…")
            record_once(args.url, args.outdir, args)
        except (urllib.error.HTTPError) as e:
            fails += 1
            log(f"HTTP {e.code}（{fails}/{args.retries}）")
        except (EOFError, OSError, ValueError) as e:
            fails += 1
            log(f"串流中斷：{e!r}（{fails}/{args.retries}）")
        if time.time() - started > 60:   # 曾穩定錄製過，重新計算重試次數
            fails = min(fails, 1)
        if stop or fails >= args.retries:
            break
        time.sleep(5)
    log("錄製結束")


if __name__ == "__main__":
    main()
