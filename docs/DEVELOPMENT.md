# 開發文件

建置、命令列用法、程式結構與授權細節。產品介紹見 [README](../README.md)。

## 建置

需要 Rust 1.88 以上（`rustup update stable` 可升級）。

```sh
cargo build --release
# 命令列執行檔在 target/release/supalive
```

`cargo build` 預設只建錄製核心和命令列。

### 桌面版

先安裝 Tauri 的命令列工具（只需一次）：

```sh
cargo install tauri-cli --version "^2" --locked
```

```sh
cd apps/desktop/src-tauri
cargo tauri dev      # 開發模式直接打開
cargo tauri build    # 打包成 .app / .dmg（macOS）或 .msi / .exe（Windows）
```

只想編譯檢查時，用 `cargo build -p supalive-desktop`。

介面是純 HTML/CSS/JS（`apps/desktop/ui`），不需要 Node.js。介面檔在編譯時嵌入，改了介面重新編譯就會帶進去。

### 視窗材質

| 平台 | 材質 | 說明 |
| --- | --- | --- |
| macOS 26 以上 | Liquid Glass | 需要 macOS 私有 API（`macOSPrivateApi`） |
| 較舊的 macOS | 毛玻璃（vibrancy） | 同上 |
| Windows 11 | Mica | Windows 10 用純色底 |

介面上的玻璃效果都由同一份 CSS 畫，兩個平台看起來一樣，只有視窗底層材質不同。設定裡的「省電模式」可關掉所有模糊效果。

### App Store 版

用 `tools/build-appstore.sh` 建置：關掉私有 API 和透明視窗，介面改用純色底，其他功能不變。

## 命令列

```sh
# 直接貼拉流網址
supalive record 'https://pull-flv-xxx.douyincdn.com/stage/stream-xxx.flv?...' -n 主播名

# 或貼瀏覽器開發者工具裡「Copy as cURL (bash)」的整段指令（指令很長時用 - 從剪貼簿讀）
pbpaste | supalive record - -n 主播名
```

同時錄多個直播間：給多個網址，`-n` 依序對應名稱。

```sh
supalive record '網址1' '網址2' -n 主播A -n 主播B
```

每個直播間各自有連線、資料夾與停止訊號，互不影響；同一直播間重複加入會被略過。程式本身不限制數量，但同時超過 2 個可能觸發抖音風控。

按 Ctrl+C 停止，最後一個切片會正常封裝，然後自動合併。

預設行為：

| 項目 | 預設 | 選項 |
| --- | --- | --- |
| 輸出位置 | `~/Downloads/SupaLive/<直播間名>/<yyyy-mm-dd-hh-mm>/` | `-o 根目錄`、`-n 直播間名` |
| 切片長度 | 約 15 分鐘，在關鍵幀處切開 | `--segment 分鐘`，0 為不切 |
| 錄完自動合併 | 開啟（直播結束或出錯都會合併已錄到的部分） | `--no-merge` |
| 合併後保留切片 | 開啟 | `--delete-clips` |
| 格式 | 分段式 MP4，中斷也不會損壞 | `--format flv` 保存原始 FLV |
| 寫檔緩衝 | 每路 16 MB，全部加起來 64 MB | `--buffer MB`、`--buffer-total MB` |

資料夾結構：

```
~/Downloads/SupaLive/主播名/2026-10-25-21-30/
├── clips/    20261025_213000.mp4、20261025_214502.mp4 …
├── cover.jpg 封面（桌面版才有）
└── merged/   主播名_2026-10-25-21-30.mp4
```

斷線會自動重連（預設連續 10 次失敗才視為結束），每次重連開新切片。解析度或編碼中途改變時無法放進同一個檔案，合併會產生 `_2`、`_3` 等多個檔案。

其他指令：

```sh
supalive remux 舊錄影.flv            # 本機 FLV 轉成 MP4（也能讀抖音的 codec 12 HEVC）
supalive merge 某次錄製/clips         # 手動合併切片
```

## 結構

| 位置 | 內容 |
| --- | --- |
| `crates/core/src/flv.rs` | FLV 拆包，支援 H.264、抖音 HEVC（codec 12）、Enhanced FLV、AAC |
| `crates/core/src/fmp4.rs` | 分段式 MP4 封裝 |
| `crates/core/src/sink.rs` | 切片、時間軸處理、原始 FLV 輸出 |
| `crates/core/src/merge.rs` | 不經 ffmpeg 合併 MP4 切片 |
| `crates/core/src/recorder.rs` | 拉流與斷線重連 |
| `crates/core/src/manager.rs` | 多直播間同時錄製、防重複 |
| `crates/core/src/session.rs` | 資料夾命名與錄完合併 |
| `crates/core/src/curl.rs` | 解析 cURL 指令 |
| `crates/core/src/live.rs` | 把錄製中的片段分送給播放器 |
| `crates/cli` | 命令列介面 |
| `apps/desktop/src-tauri` | 桌面版後端：指令、設定存檔、抖音側欄、視窗材質 |
| `apps/desktop/src-tauri/inject/douyin.js` | 注入抖音側欄、讀取頁面拉流網址的腳本 |
| `apps/desktop/ui` | 桌面版介面 |
| `spec/` | 需求與設計 |
| `tools/` | 開發用的樣本驗證腳本（用本機 ffmpeg 檢查輸出，不進發佈包）與 App Store 版建置腳本 |
| `douyin_rec.py` | 舊的 Python 原型（需要 ffmpeg），保留作參考 |

## 授權與依賴

本專案以 [GPL-3.0-or-later](../LICENSE) 授權。

不打包也不呼叫 ffmpeg，只搬移已壓縮的資料、不解碼也不編碼。依賴全部是 MIT / Apache-2.0 等寬鬆授權，可用 `cargo deny check licenses bans` 檢查（設定在 `deny.toml`）。
