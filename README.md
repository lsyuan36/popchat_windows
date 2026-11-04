# PopChat for Windows

輕量的 Windows 浮動 AI 聊天面板，以 **Rust + egui/eframe** 實作。

**Windows 移植／維護：Evis Lai** · 目前版本：**0.2.8**

> 本專案移植自 **Le Chen / lec77** 的 macOS 專案
> **[lec77/PopChat](https://github.com/lec77/PopChat)**。
> 感謝原作者的設計與開源實作。這是獨立的 Windows 移植版，並非原專案官方發行，
> 也不宣稱已完全等同原版。原始 MIT 版權聲明保留於 [LICENSE](LICENSE)。

## 下載與啟動

從 **[GitHub Releases](https://github.com/lsyuan36/popchat_windows/releases/tag/v0.2.8)**
下載 **`PopChat-0.2.8-Windows-x64-Setup.exe`**（NSIS 安裝程式）。
**0.2.8** 修正 0.2.7 設定頁無法以滾輪捲到下方欄位的問題，固定儲存列與頁面轉場維持不變。
保留 0.2.7 的圓角遮罩、小字活動提示、「關於」分頁、緊湊雙欄快捷鍵表與直接輸入待套用組合，
以及 Skills、MCP 本機安全設定、Auto Compact 與繁體中文內建提示詞。
既有版本使用者可用程式內更新，
或先結束 PopChat 再執行新版 Setup 升級。
0.2.0 Setup 的首次安裝路徑問題已於 0.2.1 修正，請優先使用新版。

1. 先從系統匣結束正在執行的 PopChat，再執行 Setup。
2. 預設安裝至 `%LOCALAPPDATA%\Programs\PopChat`，只影響目前使用者。
3. 透過桌面／開始功能表捷徑啟動，在設定中加入服務、端點與模型。

安裝程式建立 Windows「已安裝的應用程式」卸載項目，不自動啟動程式或啟用開機啟動。
卸載保留設定、服務、憑證、聊天歷史、更新備份與非安裝包的檔案。
另保留 `PopChat-Windows-x64.zip` 供便攜使用及程式內更新；使用 ZIP 時須完整解壓，
並將主程式、更新助手和授權文件放在一起。

請勿直接在 ZIP 內執行，也不需要以系統管理員身分執行。
目前提供 **Windows x64** 版本；執行檔尚未進行程式碼簽章，
Windows SmartScreen 可能顯示未識別的應用程式。請核對來源與發行頁的 SHA-256。

## 功能

- 可設定的全域快捷鍵喚出／收起、系統匣待命、視窗釘選與拖曳。
- OpenAI 相容服務與自訂模型；另有 ChatGPT 訂閱／Codex 整合。
- 串流回覆、思考內容收合、停止、重試、編輯提問、建立分支。
- 歷史紀錄、聊天搜尋、附件、貼上圖片／檔案及拖放檔案。
- Markdown、程式碼、表格與公式；訊息、程式碼及整張表格複製。
- 淺色米黃／深色主題、透明度、位置設定與頁面切換動畫。
- 中英字體分開選擇；中文預設內建 **獅尾圓體**。
- 英文預設 **已安裝的 TT Norms Pro → Arial**；這兩種字體不隨程式散佈。
- 聊天文字大小 **12–24 pt**、即時預覽；工具／搜尋活動提示固定小字，正式回答字級不變。
- 全域快捷鍵支援按鍵錄製或直接輸入，需按「套用」；聊天快捷鍵表寬版每列兩項。
- 圖示 hover 顯示功能、快捷鍵與停用原因。
- 從此 repo 的 Releases 檢查、下載更新，確認後安裝並重新啟動。

ChatGPT／Codex 整合的可用性取決於帳號、方案、外部工具及服務變更；
這不是 OpenAI 官方用戶端。不要把離線測試通過視為所有真實帳號皆已驗收。

## 常用操作

| 快捷鍵 | 功能 |
|---|---|
| `Alt+Down` | 預設全域喚出／收起，可在設定內錄製或直接輸入其他組合 |
| `Ctrl+N` / `Ctrl+Y` | 新聊天／聊天歷史 |
| `Ctrl+F` | 搜尋目前聊天 |
| `Ctrl+G` / `Ctrl+Shift+G` | 下一個／上一個搜尋結果 |
| `Ctrl+O` | 加入附件 |
| `Ctrl+E` | 展開／收合輸入框 |
| `Enter` / `Shift+Enter` | 一般輸入框送出／換行 |
| `Ctrl+Enter` | 大編輯器內送出 |
| `Ctrl+,` | 開啟／關閉設定 |
| `Ctrl+P` / `Ctrl+W` | 釘選／隱藏視窗 |

輸入法組字期間的 Enter 優先用於選字；指令補完開啟時 Enter 先選取指令。
`Alt+Space` 是 Windows 保留組合，不接受設為全域快捷鍵。
若實機無法錄製 `Alt+Left` 等組合，可直接填入待套用欄位，再按「套用」；
註冊衝突時保留原快捷鍵。錄製問題尚未在實機重現，不宣稱已修復原生輸入問題。

## 更新

設定 →「關於」中的自動檢查每天最多執行一次，也可按「立即檢查更新」；登入啟動、版本與專案連結也集中於此。
更新來源僅為 **`lsyuan36/popchat_windows` 的正式 GitHub Releases**，
不是原 macOS 專案。

有新版時先下載並驗證 GitHub 資產 SHA-256，再由使用者確認安裝和重新啟動。
更新助手等待目前程式完全退出，才替換執行檔；不會同時開兩個聊天 UI。
0.2.3 的下載進度使用 3px 圓角細線，百分比與下載量分開顯示；未知總長度時不虛構百分比。
0.2.4 的安裝確認視窗增加留白與背景遮罩、區分主次按鈕；只有明確點擊安裝才繼續，Esc 可返回。
安裝不更換資料目錄、不重匯入服務、不讀寫遷移憑證。
請在確認重新啟動前完成尚未送出的訊息與未儲存的 API 編輯。
若自動更新無法使用，仍可從 Releases 下載 ZIP 手動更新。

## 每個模型的上下文上限

設定 → 服務 → 選擇模型後，可編輯「模型上下文上限（tokens）」。
未覆寫時，GPT 系列採 **1M（1,000,000）**，其他模型採
**256K（256,000）**；這是應用程式預算，不是供應商能力保證。
同名模型在不同服務各自保存，支援正整數、`128K`、`1M`，
按「使用預設」可移除此模型的覆寫。

「儲存此模型上限」只保存該項偏好，不會提交端點／金鑰／模型選擇草稿。
這不是單次回答的 `max_tokens`。**0.2.2 沿用 0.2.1 修訂版的前景 Auto Compact**：
保留完整歷史，以摘要 checkpoint 搭配近期原文送出，並提供 usage 記錄、取消與有界超限恢復。
摘要會額外使用目前模型的額度。圖片／原始 PDF 不會被摘要丟棄；若無法安全縮減則停止送出。
Codex app-server 沿用其原生上下文管理。實作邊界與隔離驗證見 [AUTO_COMPACT.md](https://github.com/lsyuan36/popchat_windows/blob/main/AUTO_COMPACT.md)。
原先的 0.2.1 初始包只有預算設定；0.2.2 已包含完整接線，既有安裝不會因原始碼改動自行變更。

## MCP 與中文 system prompt

0.2.2 支援 MCP stdio／Streamable HTTP、服務管理與每次工具執行確認。
可以使用 `/mcp add {JSON}` 從對話開啟新增草稿；亦可在設定中允許模型以自然語言
提出草稿。**草稿不會自動保存、啟用或連線**，憑證只在設定密碼欄輸入。
聊天、摘要與各 backend 的內建指令改為繁體中文，自訂 prompt 不會被覆寫。
使用方式、安全邊界及首版限制見 [MCP.md](https://github.com/lsyuan36/popchat_windows/blob/main/MCP.md)。
0.2.1 安裝包不含 MCP；需升級至 0.2.2。Codex app-server 仍不啟用 MCP，
HTTP 首版僅支援手動 Bearer token，不提供自動 OAuth 登入。

0.2.3 補上 `mcp_status`、`open_mcp_settings` 與 `test_mcp_service`，沒有啟用服務時也能
查詢狀態、開啟設定，或經人工確認後測試已儲存但停用的服務。不開放任意 shell／curl，
不將 token 交給模型；保存／啟用仍由使用者完成，新工具於下一次送出起生效。
需要模型支援 function calling，Codex app-server 邊界不變；詳見 [MCP.md](https://github.com/lsyuan36/popchat_windows/blob/main/MCP.md)。

0.2.4 可在聊天貼入完整的 MCP 連線資訊，由**本機**擷取名稱、位址及選填憑證，
顯示將修改的設定檔路徑與待確認畫面。使用者確認新增並啟用後，接續遮罩憑證的原測試要求；
原始連線資訊不進輸入歷史或模型請求。數字內網 IP 的明文 HTTP 必須額外勾選風險同意，
公開 HTTP 仍拒絕。建議仍在本機密碼欄輸入憑證，並優先採用 HTTPS；每次工具仍須確認。

## Skills（0.2.6）

設定 → Skills → 管理技能／掃描來源，可讀取 OpenCode、Claude、Codex／共用 Agents
的個人預設路徑，也可明確指定專案或自訂資料夾。新技能預設停用，需個別啟用；
只先提供名稱與說明，模型需要時才載入全文與技能目錄內的文字參考檔。
不修改其他工具設定，不自動執行技能腳本。詳見 [SKILLS.md](https://github.com/lsyuan36/popchat_windows/blob/v0.2.8/SKILLS.md)。
**0.2.4 及更早版本不含 Skills 設定，需升級至 0.2.6。**

## 資料與隱私

- 一般資料位於 `%APPDATA%\PopChat\PopChat\config`；包含偏好、服務設定和聊天歷史。
- 金鑰／登入憑證使用 Windows Credential Manager，不應放進 repo 或 issue。
- 對模型服務、搜尋服務或登入服務的請求會依使用者設定傳送必要資料；
  更新檢查則會連線 GitHub。這不是完全離線的應用程式。
- 一般外觀偏好即時保存，**不會順帶提交未儲存的 API 端點／金鑰草稿**。
- `POPCHAT_DATA_DIR` 是開發／隔離測試選項，日常使用請勿隨意切換，
  以免看見另一份服務設定或歷史。

## 開發與驗證

需求：Windows x64、Rust stable、MSVC C++ Build Tools 與 Windows SDK。

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --all-targets --quiet
cargo build --locked --release --bins --target x86_64-pc-windows-msvc --target-dir target/button-review
```

```powershell
# 從已驗證的 release build 產生 ZIP、SHA256SUMS 和 release-manifest：
.\scripts\package-release.ps1 -SkipBuild
```

開發工作樹的使用者層級安裝／自訂服務匯入流程見 [INSTALL.md](INSTALL.md)。
新使用者不需要執行該匯入流程；使用 NSIS Setup 即可。

## GitHub Actions 打包 NSIS 安裝程式

到 **[Actions → Build Windows NSIS Installer](https://github.com/lsyuan36/popchat_windows/actions/workflows/windows-build.yml)**
按 **Run workflow**。`build_ref` 留空會建置目前分支，也可指定 tag／commit。
推送 `main`、`v*` tag 或建立 PR 時也會自動執行。

成功後，在該次工作流程頁面的 **Artifacts** 下載：

- **PopChat-Windows-x64-NSIS**：**`PopChat-版本-Windows-x64-Setup.exe`**、SHA-256 和 installer manifest。
- **PopChat-Windows-x64-Release**：程式內更新所需的 ZIP、SHA-256 和來源 manifest。

使用 GitHub 的 Windows 2022／MSVC runner 和 Rust stable，不需要自行安裝字體、
提供 API 金鑰或在 runner 開啟桌面 UI。流程下載並核對固定 NSIS 3.11 compiler SHA-256，
編譯真正的 NSIS Setup，並測試中文路徑安裝、重裝、捷徑、卸載與保留未知檔案／備份。
另驗證更新安全測試與真正的 ZIP，
不會自動發佈 Release 或修改使用者資料。Artifacts 保留 14 天；長期下載請使用 Releases。

本機打包：先執行 `scripts/package-release.ps1`，再用 NSIS 3 compiler 執行：

```powershell
.\scripts\package-installer.ps1 -PackageDirectory target/releases/v0.2.1/<產生的目錄> -MakeNsis 'C:\Program Files (x86)\NSIS\makensis.exe'
```

## 已知限制

Windows 移植仍在迭代，並非完整移植宣告。真實 provider／訂閱帳號、
原生注音、多螢幕與混合 DPI、無障礙及部分原版動畫仍需更多驗收。
文件／公式格式也不保證涵蓋所有來源。

完整函式對照與缺口：[FUNCTION_PARITY.md](FUNCTION_PARITY.md)。
實作與驗證紀錄：[PORT_PROGRESS.md](PORT_PROGRESS.md)。

回報問題請附 Windows 版本、應用程式版本、重現步驟及已去識別化的截圖；
不要上傳金鑰、聊天歷史、憑證或完整的私人設定檔。

## 授權與致謝

- 原專案：[lec77/PopChat](https://github.com/lec77/PopChat)，**Le Chen**，MIT。
- Windows Rust 移植與維護：**Evis Lai**，MIT。
- 字體、圖示及依賴授權：[NOTICE.md](NOTICE.md)、`THIRD_PARTY_NOTICES.md`。

**TT Norms Pro 與 Arial 不包含在本 repo 或發行檔中。**
