# MCP Client（0.2.6）

0.2.2 加入 MCP Client 與繁體中文內建提示詞。0.2.1 使用者需確認程式內更新或
重新下載 Setup 升級；新增設定不會自行啟用服務或啟動 MCP 程式。

## 0.2.6：介面與 Skills 整合

### MCP 與 system prompt

MCP 不會把服務全文放入 system prompt，也不會自動載入服務的 prompts。
已啟用服務每輪初始化／探索工具，工具名稱、說明與參數 schema 透過模型的 `tools`
欄位提供，因此仍會占用上下文；實際執行結果在工具回合返回後才加入本輪。
未啟用服務不會自動連線，但本機 MCP 管理工具仍會提供。
自訂 system prompt 保持獨立，不會因 MCP 或 Skills 被覆寫。
OpenAI 相容 backend 使用已設定的 system message；ChatGPT Responses 將其轉為
`instructions`，沒有 system message 時使用內建繁體中文 fallback。
Codex app-server 不注入 PopChat 的 MCP／Skills 工具，既有安全邊界維持不變。

### 視窗樣式

工具確認與服務填寫視窗改為一致的圓角卡片與欄位分組，主操作使用強調色，
取消／拒絕採次要樣式；操作列不隨內容捲動，窄視窗採直向按鈕。
工具的服務、完整目的地、執行參數與風險分區顯示；空參數改為「此工具不需要額外參數」，
非空參數不截斷。填寫頁先呈現連線與驗證欄位，已儲存服務與進階設定分開收合，
「允許模型提出新增草稿」移至「進階選項」。

這只是介面調整，不會自動保存、啟用、測試或授權工具；密碼仍遮罩，
內網 HTTP 仍需獨立同意，每次執行仍須明確確認。此介面調整需升級至 0.2.6。
畫面檢查使用假資料與離線 egui 輸出，不連線真實服務、不啟動第二個聊天 UI。
整合 Skills 後完整回歸 **620 library、29 app、1 send-pipeline integration、1 license inventory、
15 importer passed；3 ignored，合計 666 項通過**，無 warning（4 條測試執行緒）。涵蓋等寬卡片、窄視窗固定操作列、
完整長參數、密碼遮罩、真實按鈕點擊及原有授權／內網 HTTP 確認流程。
Skills 的來源、個別啟停及按需載入見 [SKILLS.md](SKILLS.md)，不影響 MCP 逐次確認邊界。

## 0.2.4：完整連線資訊的本機確認流程

本節功能自 **0.2.4** 提供，0.2.3 及更早版本需升級，不會因原始碼修改自行更新本機程式。

可將完整的結構化要求貼入聊天，例如：

```text
請幫我把下列 MCP 伺服器加進設定，並測試連線。
- 伺服器名稱:local-tools
- 傳輸方式:Streamable HTTP
- 位址:http://10.0.0.12:2918/v1/mcp

依序 initialize、tools/list，再呼叫服務提供的 capabilities 工具。
```

如果原始資訊另含獨立的「憑證:」欄，本機會在送給模型或記錄輸入歷史**之前**擷取它，
只預填在設定的遮罩密碼欄；不將原始連線全文記入聊天。非完整／模糊的憑證輸入會阻擋，
不當作普通聊天轉送。也可不貼憑證，直接在確認畫面的密碼欄輸入，這仍是建議方式。
此解析僅作用於使用者明確以「請…MCP…新增／加入／加進」開頭的結構化要求，
不會掃描助手、網頁或工具輸出來自動新增。

流程如下：

1. 本機開啟待確認畫面，顯示名稱、完整目的地、將修改的 `mcp_servers.json` 路徑，
   並明示憑證只存入本機憑證庫，不寫入該 JSON。尚未保存或連線。
2. HTTPS／localhost 沿用原規則。數字內網 IP（IPv4 RFC1918／IPv6 ULA）若使用 HTTP，
   額外顯示明文風險，必須手動勾選同意，才能按「確認新增、啟用並繼續」。
   公開網路 HTTP 仍拒絕，不能由模型提案帶入授權旗標；內網 HTTP 不使用系統 HTTP 代理。
3. 使用者確認後，保存服務並啟用，以**遮罩憑證後**的原要求接續聊天。
   runtime 先初始化並探索工具，模型可依實際工具目錄呼叫 capabilities；每次工具仍需確認。
   `test_mcp_service` 本身仍不執行工具，不會把測試權限擴大成任意 HTTP／shell。
4. 原聊天已切換時不自動送出舊要求；同名服務及未儲存草稿不自動覆寫。

內網 HTTP 的同意會綁定已保存目的地；修改 URL 須重新同意。此選項不提供 TLS、加密或
沙箱保護，Bearer token 及工具資料仍可能遭可信任網路之外的設備攔截，優先使用 HTTPS。
MCP 狀態提供原始工具名稱、安全的 HTTP 狀態碼／連線診斷，不把服務原始錯誤本文回傳模型。
已保存的已知憑證會從模型輸入及摘要的文字副本遮罩，**不回寫或刪改完整聊天歷史**。
舊版曾送出／保存的憑證無法藉此撤回，仍建議輪替。

本輪驗證只用假的憑證、隔離設定與本機 mock，不連線使用者提供的真實服務。

修正後完整回歸：**602 library、29 app、1 send-pipeline integration、15 importer passed；
3 ignored，合計 647 項通過**，無 warning。包含本機解析／輸入歷史排除、內網 HTTP 的
獨立同意與目的地變更、真實滑鼠勾選／保存、遮罩後接續請求，以及 mock 的
`initialize → notifications/initialized → tools/list → tools/call(capabilities)` 完整路徑。
安裝確認視窗另驗證淺／深色、小視窗、主次按鈕、Esc 取消與生成中提醒。

## 0.2.3：讓聊天助手管理與測試 MCP

以下管理工具自 **0.2.3** 提供；0.2.2 使用者需升級，不會因原始碼改動自行更新本機程式。

OpenAI-compatible／ChatGPT Responses 的正常聊天，會提供三個內建工具，即使沒有啟用
服務或尚未允許模型提出新增草稿，也不會只剩搜尋／讀網頁：

- `mcp_status {}`：查詢本輪服務 ID、名稱、啟停、憑證是否可讀及工具探索狀態。
  不回傳 token、secret reference、程式參數或網址，不修改設定，也不因查詢去連線。
- `open_mcp_settings {"service_id":"選填的服務 UUID"}`：開啟 MCP 設定，讓使用者新增、
  編輯、輸入 Bearer token 或啟用服務。不替使用者保存，也不覆寫尚未儲存的草稿。
  不指定服務時使用 `{}`。
- `test_mcp_service {"service_id":"已儲存服務的 UUID"}`：請求連線測試。
  **先顯示確認，獲准後才啟動／連線**，包括已保存但停用的服務。僅初始化與探索工具，
  不呼叫 `tools/call`、不保存／啟用，不把新工具動態加入本輪。

例如可說：「請查看 MCP 狀態，幫我開啟設定，再測試我已儲存的服務。」
若尚未新增，先由助手開啟設定，或使用下方 `/mcp add`。Bearer token 只在密碼欄輸入。
設定保存會停止舊工具回合；**再次送出訊息**才使用新啟用的服務工具，實際工具仍逐次確認。
自然語言新增草稿的原 opt-in 選項仍保留，不因這三個管理工具自動開啟。
「確認並儲存」固定在設定視窗底部，不隨說明文字或長服務清單一起捲動。

單一已啟用服務連線／探索失敗或憑證無法讀取，不再讓整個聊天在模型請求前中止；
失敗服務不提供工具，助手可用狀態工具診斷並引導重新設定。目錄若含已知 token 則整份拒絕，
不洩漏給模型；狀態與測試回覆也不包含 token。損壞的設定仍禁止覆寫。
Responses 工具明確採 `strict:false`，保留外部 MCP schema 的選填欄位。

這需要模型支援 function calling，**不是任意 shell／curl／HTTP 代理**；只允許測試
使用者已保存的目的地，不接受工具參數中的新 URL 或 token。Codex MCP 禁用邊界不變，
Auto Compact 摘要回合也不提供這些管理工具。本輪未連線使用者的真實服務或模型。

修正後 fmt／all-target check／diff check 通過，無 warning；完整回歸：
**594 library、28 app、1 send-pipeline integration、15 importer passed；3 ignored**，
合計 **638 項通過**。涵蓋無服務時的管理入口、OpenAI／Responses 工具回合、
停用服務測試的准許／拒絕／取消、舊回合與設定快照 gate、實際確認按鈕／Esc，
以及小視窗長服務清單的固定儲存按鈕點擊、淺／深色的 3px 更新進度與未知下載總量。

## 從對話新增

### 不依賴模型的明確指令

在聊天輸入：

```text
/mcp add {"name":"我的遠端工具","transport":{"type":"http","url":"https://example.com/mcp"}}
```

或本機程式：

```text
/mcp add {"name":"我的本機工具","transport":{"type":"stdio","command":"node.exe","args":["D:\\Tools\\my-mcp\\server.js"]},"token_env":"API_KEY"}
```

這是 PopChat 本機處理的命令，不會交給模型，也不會把原始 JSON 記入送出歷史。
只開啟待確認的 MCP 草稿，預設**停用**，不保存、不連線、不執行或安裝任何程式。
JSON 無效時保留編輯草稿，顯示錯誤，不作一般聊天重送。

請在 MCP 視窗檢查名稱、網址／程式與 args，必要時於密碼欄輸入 token，
再按「確認並儲存 MCP 服務」。勾選「啟用」才授權在下一次聊天送出時連線／啟動。
同名新增不會覆寫既有服務；既有服務必須由使用者明確選取編輯。

### 自然語言提出草稿

在「設定 → MCP → 管理 MCP 服務」開啟：

> 允許模型提出 MCP 新增草稿（須支援工具呼叫）

然後可說：「請幫我加入名為筆記工具的 MCP，網址是 https://example.com/mcp」。
模型可呼叫內建 `propose_mcp_service` 建立草稿，**依然必須由使用者確認**。
此選項預設關閉；`/mcp add` 不受此限制。0.2.3 的三個管理工具不受此選項限制，
但管理／提案及 MCP 工具呼叫均需要模型支援 function calling。
只解析明確使用者命令或此結構化提案工具，不掃描助手文字／網頁／工具輸出自動新增。
新提案不會覆寫使用者正在編輯的草稿，待檢視佇列最多保留 4 份。

**不要在聊天、URL、程式參數或提案中貼上金鑰。** 對話提案不接受 `enabled`、
`id`、`headers`、`env` 或 token 值；常見的明文憑證 CLI 參數也會被拒絕。

## 服務管理與工具權限

- 設定提供新增、編輯、刪除與連線測試；測試按鈕明確標示會啟動程式／連線。
- 只有已啟用服務會在聊天送出時初始化；App 啟動時不自動連線。
- 連線測試只做 `initialize → notifications/initialized → tools/list`，不做 `tools/call`。
- 模型工具以服務 UUID＋工具名稱摘要產生別名，避免服務間名稱碰撞或冒充內建工具。
- **每次** MCP 工具執行先顯示服務、目的地、工具及完整參數，使用者只能「拒絕」
  或「僅允許這一次」。不因 MCP `readOnlyHint`、工具敘述或模型文字自動授權。
- 拒絕／逾時不執行工具；服務設定已改變、聊天／回合已切換的舊確認請求也不會執行。
- 同一回合拒絕過的工具不再重複彈出確認。
- Stop 會中止等待確認、連線及工具回合。本機服務綁定 Windows kill-on-close Job Object，
  關閉 session 時結束自己的程序樹，不搜尋／終止其他同名使用者程序。
- HTTP 會盡力發送取消通知並結束 session；**停止不能保證撤銷遠端已完成的副作用**。
- 設定變更會停止進行中的聊天工具回合，新設定於下一輪生效。

> **MCP 不是沙箱。** 本機 stdio 程式以你的使用者權限執行，從啟動時就可能讀寫檔案
> 或連網；工具執行前的確認不限制程式啟動本身。`npx` 等啟動器也可能依你批准的參數
> 下載套件。只啟用受信任的程式與服務。PopChat 不自行下載或安裝 MCP 套件。

## 協定與實作範圍

- **stdio**：UTF-8 JSON-RPC 換行框架、初始化／版本協商、分頁工具探索及呼叫。
  command／args 直接傳給程序，不拼接 shell 指令。Windows 的一般 `npx` 會解析成
  相鄰的 `node.exe + npx-cli.js`；不直接執行 `.cmd`／`.bat`。
- **Streamable HTTP**：POST 支援 JSON 與 SSE 回覆、多行 SSE data、session ID、
  negotiated protocol header、初始化通知、取消及最佳努力 DELETE session。
- 優先協商 `2025-11-25`，另接受 `2025-06-18`／`2025-03-26`；stdio 另接受 `2024-11-05`。
- HTTP 使用 HTTPS，localhost 可 HTTP；0.2.4 另允許明確同意的數字內網 HTTP。
  不跟隨 redirects，不接受 URL 帳密／query／fragment。
  認證首版只支援手動 Bearer token，無自動 OAuth 登入或 token 刷新。
- 首版範圍為 MCP **tools**；不提供 resources／prompts／sampling／elicitation／roots／tasks。
  不支援的 server-to-client requests 回覆 method-not-found，不會替服務取用檔案或呼叫模型。
- 尚未支援舊式 HTTP+SSE 雙端點 transport 或 SSE 斷線續傳；失敗時不重送 `tools/call`。
  session 過期後本輪停止，後續新聊天送出會建立新 session。
- 服務數最多 16；單服務工具分頁最多 8 頁，全目錄最多 64 個工具／64 KiB。
  0.2.3 保留 3 個管理工具的配額，外部工具與選擇性的新增草稿工具合計最多 61 個。
  JSON-RPC 框架／回覆最多 1 MiB，工具參數最多 16 KiB，文字結果最多 32 KiB。
  網路／RPC 期限 60 秒，使用者確認期限 180 秒，本輪最多 32 次 MCP 工具／提案。
- MCP 結果只傳文字／structuredContent；二進位內容明確標示省略，已知認證 token 會遮罩。
  若服務把認證 token 放進工具目錄，整個目錄會被拒絕，不把該 token 傳給模型。
- OpenAI-compatible 與 ChatGPT Responses 都支援工具回合；需要模型本身支援 function calling。
  Codex app-server 仍禁止 MCP，不解除它原有的只讀／工具安全邊界。
- Auto Compact 的摘要回合永不開啟 MCP，避免背景摘要順便連線或提出服務草稿。

## 設定與憑證

`mcp_servers.json` 存在聊天 store 的 profile 目錄，正常為
`%APPDATA%\PopChat\mcp_servers.json`，隔離模式則使用 `POPCHAT_DATA_DIR`。
只保存服務設定與隨機 secret reference。token 分片保存在既有 `SecretStore`／Windows
Credential Manager，驗證片段 SHA-256；設定原子寫入失敗時不替換舊憑證。
不使用模型服務的 API key 作 MCP 認證，也不把該 key 傳給 MCP 子程序環境。
讀取損壞的設定時停用 MCP 並禁止覆寫原檔。

## 繁體中文提示詞

聊天預設、摘要、ChatGPT fallback、Codex developer 邊界與內建工具說明已改繁體中文。
工具／協定欄位、識別碼、`pasteable` 標籤及外部服務提供的原文保留，不任意翻譯。
只有與舊內建英文 system prompt **完全相同**的設定，在載入時採用新中文預設；
自訂 system prompt／已儲存指令不會被覆寫。自訂內容若也要換回中文預設，
可在「設定 → 指令 → 使用預設」切換，僅保存 prompt，不提交未儲存的 API 草稿。

## 驗證

測試使用本機 mock HTTP、隔離設定／file-only 假憑證及自己建立的 stdio fixture，
不連線真實模型或 MCP、不讀取使用者金鑰、不啟動第二個聊天 UI。

```powershell
cargo test --locked --lib chat::mcp::tests -- --nocapture
cargo test --locked --lib ui::mcp::tests -- --nocapture
cargo test --locked --all-targets
```

0.2.2 發佈時 fmt／all-target check／diff check 通過，無 warning；all-target tests：
**583 library、27 app、1 send-pipeline integration、15 importer passed；3 ignored**，
合計 **626 項通過**。包含兩個 backend 的 MCP 工具回合、自然語言提案不連線、
stdio 子程序清理、實際 UI 儲存按鈕／Esc 拒絕、舊回合拒絕、token 分片與遮罩，
以及中文預設遷移不覆寫自訂 prompt 等測試。

規格依據：
- https://modelcontextprotocol.io/specification/2025-11-25/basic/transports
- https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle

這不是所有真實 MCP server／OAuth／第三方工具的相容性宣告；尚需使用者指定服務的個別驗收。
