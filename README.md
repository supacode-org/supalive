# SupaLive

抖音直播錄製助手。直接保存直播的原始串流：不錄屏、不重新編碼，也不需要 ffmpeg。

有兩種用法：桌面版（Windows / macOS，Tauri 2）和命令列工具。兩者共用同一個錄製核心。

## 建置

需要 Rust 1.88 以上（`rustup update stable` 可升級）。

```sh
cargo build --release
# 命令列執行檔在 target/release/supalive
```

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

介面是純 HTML/CSS/JS（`apps/desktop/ui`），不需要 Node.js。

桌面版功能：

- 貼上直播間網址（或抖音 App 的分享文字），右側的「選取直播」會打開抖音網頁版跳過去，抓到各種畫質讓你選一個再錄；也可以在裡面登入後自己逛到直播間
- 多個直播間同時錄製，同時超過 2 個時提醒（可在設定關掉，程式本身不限制）
- 邊錄邊播：播放器直接接收正在寫入的片段，可往回拖最近約 3 分鐘
- 錄完自動合併、保留分段、分段長度等設定與命令列版相同

視窗材質：

| 平台 | 材質 | 說明 |
| --- | --- | --- |
| macOS 26 以上 | Liquid Glass | 需要 macOS 私有 API（`macOSPrivateApi`） |
| 較舊的 macOS | 毛玻璃（vibrancy） | 同上 |
| Windows 11 | Mica | Windows 10 用純色底 |

介面上的玻璃效果都由同一份 CSS 畫，兩個平台看起來一樣，只有視窗底層材質不同。
設定裡的「省電模式」可關掉所有模糊效果。

將來如果要上架 App Store，用 `tools/build-appstore.sh` 建置：會關掉私有 API 和透明視窗，介面改用純色底，其他功能不變。

已知限制：

- HEVC 直播在 Windows 上播放需要安裝「HEVC 視訊延伸模組」；錄製本身不受影響
- Linux 不是目標平台：可以編譯執行，但「選取直播」的位置和已完成檔案的播放不正常

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

每個直播間各自有連線、資料夾與停止訊號，互不影響；同一直播間重複加入會被略過。
程式本身不限制數量，但同時超過 2 個可能觸發抖音風控。

按 Ctrl+C 停止，最後一個切片會正常封裝，然後自動合併。

預設行為：

| 項目 | 預設 | 選項 |
| --- | --- | --- |
| 輸出位置 | `~/Downloads/SupaLive/<直播間名_yyyy-mm-dd-hh-mm>/` | `-o 根目錄`、`-n 直播間名` |
| 切片長度 | 約 15 分鐘，在關鍵幀處切開 | `--segment 分鐘`，0 為不切 |
| 錄完自動合併 | 開啟（直播結束或出錯都會合併已錄到的部分） | `--no-merge` |
| 合併後保留切片 | 開啟 | `--delete-clips` |
| 格式 | 分段式 MP4，中斷也不會損壞 | `--format flv` 保存原始 FLV |

資料夾結構：

```
~/Downloads/SupaLive/主播名_2026-10-25-21-30/
├── clips/    20261025_213000.mp4、20261025_214502.mp4 …
└── merged/   主播名_2026-10-25-21-30.mp4
```

斷線會自動重連（預設連續 10 次失敗才視為結束），每次重連開新切片。
解析度或編碼中途改變時無法放進同一個檔案，合併會產生 `_2`、`_3` 等多個檔案。

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
| `tools/` | 開發用的樣本驗證腳本（用本機 ffmpeg 檢查輸出，不進發佈包）與 App Store 版建置腳本 |
| `douyin_rec.py` | 舊的 Python 原型（需要 ffmpeg），保留作參考 |

## 授權與依賴

本專案以 [GPL-3.0-or-later](LICENSE) 授權：可以自由使用、修改、散佈與商用，但散佈修改後的版本時必須以相同授權公開原始碼。

不打包也不呼叫 ffmpeg，只搬移已壓縮的資料、不解碼也不編碼。
依賴全部是 MIT / Apache-2.0 等寬鬆授權，可用 `cargo deny check licenses bans` 檢查（設定在 `deny.toml`）。
