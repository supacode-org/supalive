---
name: release
description: 在本機打包 SupaLive 的 macOS 與 Windows（arm64、x86_64）安裝檔並發佈到 GitHub Release，發佈後版本號自動加一。Leo 說要發佈、發版、出 release 時使用。
---

# 發佈 SupaLive

規則見 `spec/20261101-release.md`，指令都在 `tools/release.sh`。

## 1. 先問 Leo

推 tag、建 Release、推版本號提交都是 git 寫入和發佈。動手前跟 Leo 確認這次的版本號（`Cargo.toml` 的 `[workspace.package] version`），他同意才繼續。要升次版本號或主版本號時，先改 `Cargo.toml`、提交推送，再開始。

## 2. 打包

```sh
tools/release.sh build
```

- 要跑十幾分鐘以上，放到背景執行，等它結束。
- 會先檢查：在 `main`、工作區乾淨、和 `origin/main` 同步、tag 不存在、工具都裝了。缺工具時照 spec「本機需要的工具」補，安裝前先問 Leo。
- 失敗時看錯誤修好，修正要另外提交（照 AGENTS.md 的規則問 Leo），再重跑。

完成後產物在 `dist/v<版本>/`，共八個檔案。請 Leo 先裝 Apple Silicon 的 dmg 試一下；Windows 版要在 Windows 機器上試。

## 3. 寫 Release 說明

在 scratchpad 寫一份 Markdown（正體中文），不要放進倉庫：

- 這版的重點：用 `git log v<上一版>..HEAD --oneline` 整理，寫給使用者看，不寫實作細節。第一版就寫主要功能。
- 下載對照表：哪個平台、哪個架構下載哪個檔案。
- 安裝提醒：macOS 第一次打開要到「系統設定 → 隱私權與安全性」按「強制打開」；Windows SmartScreen 按「其他資訊 → 仍要執行」。
- 照 AGENTS.md 的隱私規則檢查：沒有本機路徑、帳號、真實直播間。

把說明給 Leo 看過再發佈。

## 4. 發佈

```sh
tools/release.sh publish <說明檔>
```

它會推 tag、建立 Release 並上傳八個檔案，然後把修訂號加一、提交「版本號改為 x.y.z」並推上 `main`。

完成後用 `gh release view v<版本> --repo supacode-org/supalive` 確認八個檔案都在，把 Release 連結給 Leo。

## 出錯時

- tag 推了但 Release 沒建成：修好後用 `gh release create` 補建，不要重推 tag。
- 要整個重來：先問 Leo，再刪掉 Release 和遠端、本機的 tag。
