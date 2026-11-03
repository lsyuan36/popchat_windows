# v0.2.3

- 程式內更新下載進度改為 3px 圓角細線，百分比與下載量移到色條外，避免沿用設定控制項的 32px 高度；未知總長度時不虛構百分比。
- 正常聊天即提供 `mcp_status`／`open_mcp_settings`／`test_mcp_service`，不再因沒有啟用 MCP 或沒開啟模型草稿而完全缺少管理入口；需模型支援 function calling，Codex 邊界不變。
- 已儲存的停用服務也可由助手請求連線測試，但須獨立人工確認；只握手／探索，不啟用、不執行工具、不接受任意 URL／token。
- 連線／探索或憑證失敗的服務不阻斷整輪聊天，保留本機診斷工具；不洩漏服務錯誤內容或 token。
- Responses 明確使用非嚴格 function schema，避免改變 MCP 選填欄位。
- MCP 設定的儲存按鈕移到捲動區外的固定底部，避免說明文字／多個服務把確認按鈕裁切。
- 本機完整回歸：594 library、28 app、1 send-pipeline integration、15 importer passed；3 ignored（638 項通過）。

# v0.2.2

- 新增 MCP Client：stdio 與 Streamable HTTP，含服務管理、工具探索與連線測試。
- 支援 `/mcp add {JSON}` 與選擇性自然語言提案，僅建立停用的待確認草稿，不自動保存、啟用、連線或安裝。
- 每次 MCP 工具執行須使用者確認；拒絕、取消、逾時、舊回合或服務設定變更不會執行。
- OpenAI-compatible／ChatGPT Responses 接入 MCP 工具回合；Codex 維持原有 MCP 禁用邊界。
- Token 使用 SecretStore 分片／digest，設定 JSON 只有 reference；失敗寫入不替換舊憑證，已知 token 不回傳模型。
- 聊天、摘要、ChatGPT fallback、Codex 邊界與內建工具說明改繁體中文；舊英文預設精確匹配時轉中文，自訂／空白 prompt 保留。
- 沿用 Auto Compact、原圖示、安全 NSIS 安裝與完整授權文件；未新增 Cargo 依賴。
- 本機回歸：583 library、27 app、1 send-pipeline integration、15 importer passed；3 ignored（626 項通過）。

MCP 首版限 tools，HTTP 僅手動 Bearer token；不含 OAuth、舊 HTTP+SSE、resources／prompts／sampling。
stdio 程式不是沙箱，可能自啟動時操作電腦；只啟用受信任服務。詳見 `MCP.md`。
Windows 執行檔仍未簽章；來源 commit 與 SHA-256 以 CI manifest 為準。

# v0.2.1

## 2026-11-03 修訂重發：Auto Compact

- 將模型預算接入實際送出：近上限先摘要舊文字，以 checkpoint＋近期原文送出，完整聊天歷史保留。
- 摘要固定以明確 user 任務結尾，避免相容 API 把末尾助手訊息當作續寫；拒絕空白、未完整結束、過長及僅複述助手回覆的摘要。
- 加入 checkpoint／usage 原子保存、重新載入、前綴指紋驗證、取消與保存失敗回復。
- 結構化 context overflow 僅在尚未輸出或使用工具時嘗試一次恢復；不無限重送。
- 圖片／原始 PDF 不會當成文字摘要丟棄；無法安全縮減時停止送出。Codex app-server 使用原生 context 管理。
- 隔離回歸：562 library、26 app、1 send-pipeline integration、15 importer passed；3 ignored。

**版本仍為 0.2.1**。已安裝原先 0.2.1 的使用者需重新下載 Setup；相同版本不會自動提示更新。
來源 commit 與 SHA-256 以修訂包 manifest 為準。詳見 `AUTO_COMPACT.md`。

## 原始 0.2.1 變更

- 修正 NSIS 首次安裝至不存在的資料夾時，路徑可能清空並寫向磁碟根目錄的問題。
- 路徑解析失敗、非法位置、檔案佔用目的地或 junction 一律停止，不回退到其他目錄。
- 每次設定輸出目錄和解壓檔案後立即檢查錯誤；新增卸載位置識別，避免搬動卸載程式後誤刪其他資料夾。
- 安裝回歸改為不預建目標與父資料夾，涵蓋中文新路徑、重裝、無效目的地及卸載保留未知檔案／備份。
- 將既有雙氣泡托盤圖示共用於視窗、EXE、捷徑、Setup 及卸載程式，提供 16–256px 圖示。
- 移除開發版通道標示；保留作者 Evis Lai、原專案及 MIT／第三方授權說明。
- 新增每服務／模型的上下文上限設定：GPT 預設 1M，其他 256K，支援個別覆寫與還原預設。
  儲存上限不提交 API 草稿；原始包尚未啟用的 auto compact 已在上述修訂包接入。

0.2.1 NSIS 安裝程式由 GitHub Actions 建置；推送原始碼不會自動建立 GitHub Release。
Windows 執行檔仍未簽章，SHA-256 不是發行者數位簽章。

# v0.2.0

Windows Rust 移植／維護：**Evis Lai**。
原專案：[lec77/PopChat](https://github.com/lec77/PopChat)，作者 **Le Chen**，MIT。

## 本版重點

- 新增正式專案 README、原作者標記與完整的發行授權文件。
- 更新來源改為 `lsyuan36/popchat_windows` 的 GitHub Releases。
- 支援程式內下載更新，驗證 GitHub 資產 SHA-256，經使用者確認後以獨立
  Rust 更新助手等待目前程式退出、安裝並重新啟動；保留偏好、服務與歷史。
- 發行檔為 Windows x64 ZIP，包含聊天程式、更新助手及第三方授權聲明。
- 補充 NSIS Setup.exe 安裝包：目前使用者安裝、捷徑、Installed apps 卸載項目，
  保留設定／歷史／憑證。Actions 直接編譯 NSIS，ZIP 仍保留給程式內更新。
- 內建獅尾圓體中文預設；英文預設優先使用已安裝的 TT Norms Pro，再使用 Arial。
  **不散佈 TT Norms Pro 或 Arial。**
- 新增 12–24 pt 聊天文字大小、快捷鍵錄製、分組鍵帽式快捷鍵清單。
- 圖示 hover 顯示功能與快捷鍵，停用狀態顯示原因。
- 加入頁面／設定子分頁轉場與視窗高度動畫。
- 改善訊息工具列、緊湊間距、表格欄寬／列高與整表 Markdown 複製。

## 下載

建議下載 **`PopChat-0.2.0-Windows-x64-Setup.exe`**，先結束系統匣的 PopChat 再安裝。
不需管理員權限，不會自動啟動程式或啟用開機啟動；卸載保留使用者資料與備份。

也可下載 **`PopChat-Windows-x64.zip`**，完整解壓縮後執行 `popchat_windows.exe`。
`popchat_updater.exe` 必須保留在同一資料夾；不需系統管理員權限。
`SHA256SUMS.txt` 與 `release-manifest.json` 提供檔案雜湊和來源 commit。

## 注意事項

- 這是獨立、未簽章的 Windows 移植版，並非原 macOS 專案的官方發行。
- SHA-256 驗證是完整性檢查，不是發行者數位簽章。
- 重新啟動前請完成尚未送出的訊息與未儲存的 API 草稿；不會擅自提交它們。
- Windows 原生輸入法、完整混合 DPI／無障礙，以及所有真實供應商帳號的驗收仍未完成。
- 新增更新流程有離線與隔離檔案交易測試；本版是第一個公開發行版本，
  尚無更高版本可供真實帳號的「下一版更新」驗收。
