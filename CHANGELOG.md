# v0.2.5

- 新增 Skills 設定與本機來源管理：OpenCode、Claude、Codex／共用 Agents 預設路徑、舊版 Codex 相容目錄，以及明確指定的專案／自訂資料夾；個別啟停並保存於獨立的 `skills.json`，不修改其他工具設定。
- Skills 接上 OpenAI 相容及 ChatGPT Responses 的實際工具回合；只提供已啟用技能的 metadata，透過 `load_skill`／`read_skill_file` 按需載入指引及目錄內文字參考檔，不自動執行腳本或授予 MCP 權限。
- 技能預設停用，內容變更需重新啟用；限制路徑、檔案大小、掃描深度與工具目錄預算，拒絕符號連結／junction，設定變更會停止舊工具回合。Codex app-server 邊界維持不變。

- 統一 MCP 工具確認與服務填寫視窗的卡片、留白、字級及背景遮罩；主次操作固定於底部，窄視窗改用直向按鈕，避免擠壓或裁切。
- 工具確認分開顯示服務、目的地、參數與風險提示；空參數以文字說明取代 `{}`，非空參數保留完整內容供捲動檢視。
- 服務填寫改為連線資訊、驗證、啟用與風險分組；密碼維持遮罩，已儲存服務與進階選項獨立收合，保留原有逐次授權與內網 HTTP 同意流程。
- 離線預覽檢查 MCP／Skills 的淺／深色與窄視窗；整合完整回歸以 4 條測試執行緒執行，666 項通過、3 項忽略，無 warning，未連線真實服務或改動本機設定。
- 補齊新增 YAML 依賴的完整第三方授權與雜湊檢查，整合於既有通知檔；保留 877-entry ZIP 布局與真正 NSIS 安裝程式，成品來源及 SHA-256 以對應 tag 的 CI manifest 為準。

# v0.2.4

- 重新設計安裝確認視窗：增加留白、加深背景遮罩，主操作使用強調色並靠右，底部按鈕不隨內容捲動；生成中提醒只在實際生成時顯示，修正「立即停止」的不精確描述。
- 支援在聊天貼入完整的 MCP 連線資訊，由本機解析名稱／位址與憑證，先顯示設定檔路徑與待確認畫面；確認新增並啟用後，接續已遮罩的原測試要求，不必再次輸入「已啟用」。
- 這類連線資訊與 `/mcp` 命令不寫入輸入歷史；憑證只進本機憑證庫，不交給模型或寫入 MCP 設定 JSON。已知 MCP 憑證亦會從送模型的舊訊息／摘要副本遮罩，原始歷史不覆寫。
- 數字內網 IP 的 HTTP 可由使用者逐一確認明文傳輸風險後使用，預設禁止；改目的地須重新同意，不接受公開 HTTP，也不把內網憑證交給代理伺服器。
- MCP 狀態補上實際工具名稱與安全診斷（如 HTTP 401／403），不回傳服務原始錯誤或憑證；工具執行仍須逐次確認。
- 本機完整回歸：602 library、29 app、1 send-pipeline integration、15 importer passed；3 ignored（647 項通過），無 warning。

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
