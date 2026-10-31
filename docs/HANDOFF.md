# SupaLive 交接文件

更新於 2026-10-25。前兩個階段在 Claude 的 Projects 模式下完成，這份文件記下所有背景，方便之後在一般 Claude Code 裡接手。

## 目標

- 抖音直播錄製助手。直接保存直播的原始串流，不錄屏，也不重新編碼。
- 兩種輸入方式：
  - 貼上拉流網址或 cURL 指令；
  - 在側欄登入抖音後進入直播間，自動抓到拉流網址。
- 目前只支援抖音。
- 平台以 Windows 和 macOS 桌面版為主。行動端要成本不高才做（Android 之後可以考慮，iOS 受背景限制，不建議做）。
- 純開源、不盈利。暫時沒有上架商店的計畫，但不排除。

## 約束

- 發佈包要特別避開 ffmpeg 這類有智慧財產權風險的依賴。專案不盈利，但不想受制於人。最好自己實作；不得不用時，讓使用者自己安裝，或者提供安裝指令。
- macOS 的透明視窗目前可以用 `macOSPrivateApi`，但要放在開關後面，方便將來上架時關掉。
- 介面在兩個平台要一致，只有原生視窗外框可以不同。玻璃效果由共用的 CSS 實現，不使用 SVG 折射。
- 設計時就要支援多個直播間同時錄製，並處理好彼此的衝突。介面在同時超過 2 個時提醒可能觸發封控，底層不設上限。
- 希望能邊錄邊播，做一個簡單版的播放器就行。

## 錄製預設值

| 項目 | 預設 |
| --- | --- |
| 分段長度 | 可設定，預設 15 分鐘。不是精確值，會在關鍵幀或串流斷開處切 |
| 錄製結束後自動合併 | 開（不論是出錯中斷還是直播結束） |
| 合併後保留分段 | 開 |
| 輸出位置 | `~/Downloads/SupaLive/<直播間名>/<yyyy-mm-dd-hh-mm>/`，讀不到名稱時放在 `未命名直播間/`；底下有 `clips/` 和 `merged/`，合併檔叫 `直播間名_時間.mp4` |

Leo 原本寫的是 `yyyy-m-d-h-m`，實作時補了零，方便排序。2026-10-26 依 Leo 的要求從「直播間名_時間」一層改成「直播間名/時間」兩層，舊的錄影留在原處不搬。

## 參考資料

- 技術棧評估、液態玻璃調研、Leo 確認過的介面原型：存在 Leo 的私人頁面，不隨程式公開。結論已經寫進這份文件和程式裡。
- 舊的 Python 原型 `douyin_rec.py`：把 FLV 裡 codec id 12 的 HEVC 改寫成 Enhanced FLV 的 hvc1，再交給 ffmpeg `-c copy` 分段。現在只作參考。

## 架構

選用 Tauri 2 + Rust，用系統 WebView，安裝包不帶 ffmpeg。容器轉換自己寫，把 FLV 轉成分段式 MP4。

| 位置 | 內容 |
| --- | --- |
| `crates/core` | 錄製核心：FLV 拆包（H.264、抖音 codec 12 HEVC、Enhanced FLV、AAC）、SPS 解析、fMP4 封裝、分段、斷線重連、不經 ffmpeg 的合併、多路管理（`manager.rs`，同一直播間防重複）、邊錄邊播分送（`live.rs`）、cURL 解析 |
| `crates/cli` | 命令列：`record`、`remux`、`merge` |
| `apps/desktop/src-tauri` | 桌面版後端：指令、設定與直播間存檔（`store.rs`，存在 App 設定資料夾的 `supalive.json`）、視窗材質（`material.rs`）、播放器通道（`player.rs`）、抖音側欄（`douyin.rs`） |
| `apps/desktop/src-tauri/inject/douyin.js` | 注入抖音側欄的腳本。它攔截頁面自己的 fetch/XHR 和 `webcast/room/web/enter` 的回應，把拉流網址回報給主介面 |
| `apps/desktop/ui` | 介面，純 HTML/CSS/JS，沒有 npm |

重點設計：

- **邊錄邊播**：核心每寫完一段 fMP4 片段，就透過 Tauri 的 IPC Channel 用二進位格式送給介面。格式是 `0` 加 init 段，或 `1` 加片段，或 `2` 表示結束。介面用 MSE 播放。後端保留約 180 秒、最多 96 MB 的緩衝，所以可以往回拖大約 3 分鐘。跟播時和最新畫面的距離會依片段長度調整。
- **權限隔離**：capabilities 用 `webviews` 區分。抖音側欄的子 WebView 只能呼叫 `douyin_stream`，主介面則拿不到這個指令。
- **視窗材質**：
  - macOS 先試 Liquid Glass，失敗就退回 vibrancy（Sidebar）；
  - Windows 用 Mica；
  - 拿不到材質時用純色底；
  - 設定裡有「省電模式」，可以關掉所有模糊效果。
- **App Store 版**：`tools/build-appstore.sh` 會套用 `tauri.appstore.conf.json`，關掉私有 API 和透明視窗。tauri-cli 建置時會改寫 `Cargo.toml`，腳本建完會還原。
- **播放已完成的錄影**：透過 asset protocol，範圍是輸出根目錄。

## 進度

| 提交 | 內容 |
| --- | --- |
| `a8d360e` | 第 1 階段：Rust 錄製核心與 CLI |
| `2133d43` | 標註最低 Rust 版本 1.88 |
| `62f2075` | 第 2 階段：Tauri 桌面版、邊錄邊播、抖音側欄抓流、App Store 建置腳本 |
| `6d6d3c2` | 側欄與抖音面板可拖曳調整寬度（雙擊恢復預設，寬度記在本機） |

已驗證：

- 在 Leo 的 Mac 上錄了一場真實直播 30 秒，是 HEVC 720x1270 加 AAC 的 FLV：
  - 輸出標成 hvc1；
  - ffmpeg 完整解碼沒有報錯；
  - macOS 能讀出長度和解析度。
  - 檔案比實際錄製時間長約 3 秒，是 CDN 剛連上時先送的緩衝。
- 用 H.264 和抖音式 HEVC 樣本測過：分段與合併檔都能完整解碼，分段接縫處音畫同步，3 路並行也正常。
- 桌面版在 Linux 測試環境用模擬串流驗證過：
  - 錄製、多路同時錄、邊錄邊播跨分段、往回拖；
  - 停止後自動合併、手動合併；
  - 新增與設定對話框、拖曳調整寬度。
- 在 Leo 的 Mac 上（macOS 27）：
  - debug 版編譯沒有錯誤和警告，App 能打開；
  - App Store 版也能編譯，建完 `git status` 是乾淨的。

## 待在 Mac 上確認

1. 視窗有沒有 Liquid Glass 或 vibrancy 效果。
2. 抖音側欄能不能登入、進直播間後會不會抓到拉流網址，以及拖曳寬度時原生 WebView 會不會跟著移動。
3. WKWebView 能不能用 MSE 邊錄邊播 HEVC。
4. 能不能播放已完成的錄影。Linux 的 WebKitGTK 不能從自訂協定播放影片，所以這一項還沒驗證。
5. 關閉視窗時，正在錄的會先正常結束、合併再退出。

## 已知限制

- 合併只支援 MP4 分段，`--format flv` 輸出的分段不能合併。
- 純音訊的直播間暫時不支援。
- 解析度或編碼中途改變時，合併會產生 `_2`、`_3` 等多個檔案。
- 錄製卡片上的縮圖目前只是漸層佔位。
- Windows 播 HEVC 需要安裝「HEVC 視訊延伸模組」，錄製本身不受影響。
- 還沒在 Windows 上建置過。
- Linux 不是目標平台：抖音側欄的位置不對，也不能播放已完成的檔案。

## 接下來

1. 在 Mac 上完成上面的確認，修掉發現的問題。
2. 在 Windows 上建置並試用（Mica、自繪視窗按鈕、WebView2 播放）。
3. 錄製卡片顯示真實縮圖。
4. 打包與簽名（`cargo tauri build`）。
5. 行動端暫不處理。

## 測試小技巧

- 雲端環境連不到抖音 CDN，真實直播只能在 Mac 上測。
- 開發時可以用本機的 HTTP 伺服器，以接近即時的速度送 FLV 樣本，模擬直播。`tools/check_samples.sh` 會用本機 ffmpeg 檢查輸出；`tools/to_codec12.py` 能把 HEVC FLV 改成抖音的 codec 12 格式。
- `~/Downloads/SupaLive/raw_sample.flv` 是一段 15 秒的真實原始串流，留著除錯用。
