# Auto Compact（0.2.1 修訂重發包）

原始 0.2.1 只有模型上下文上限偏好。以下描述 0.2.1 修訂重發包中的送出流程，
既有 0.2.1 安裝需重新下載 Setup；相同版本不會自動提示更新。
這不表示所有模型已完成真實服務驗收。

## 分析與修正

1. 原先 `ChatStore::send()` 沒有把模型預算交給網路送出流程；現在
   `OutgoingTurn.context` 攜帶實際 provider UUID、model 上限與聊天 checkpoint，
   GUI 的 `start_stream()` 經 `chat::compaction::run()` 執行。
2. 實測相容 API 可能把末尾助手訊息視為續寫前綴，摘要只得到一段確認文字。
   摘要現在固定包含專用 system 指令，以及**最後一則明確的 user 摘要任務**。
3. 摘要成功後，僅以「checkpoint＋近期完整原文」重建模型請求；原始 UI 訊息、
   附件、Copy、搜尋、歷史檔與 fork 仍保留完整資料，不用摘要覆寫它們。

## 觸發及預算

- 原有預設仍為 GPT 1,000,000、其他 256,000；provider／model 可獨立覆寫。
- 保留至少 15% 的 context 作輸出與餘裕；小 context 的最低保留量按比例縮小。
  例如 1M 模型的輸入門檻為 850K，256K 為 217.6K。
- OpenAI-compatible SSE usage 與 ChatGPT Responses usage 會記錄於該聊天。
  同一 provider/model 且請求前綴未變時，下一輪預測會納入服務回報的 input/output。
- 沒有 usage 或前綴／模型改變時使用文字估算；ASCII 與非 ASCII 採不同權重。
  **這不是精確 tokenizer，也不是服務商容量保證**。圖片／PDF 成本只能粗估。
- 近期原文從 user 回合邊界開始，較大 context 保留至多約 15K 估算 tokens，
  小 context 按比例縮減；目前 user 請求絕不納入摘要。
- 舊文字分段摘要，每段接續前一份 checkpoint，單次送出最多 32 段，摘要每段
  有 180 秒期限。摘要不啟用 web、function tools 或 plugins。

## 保存、取消與錯誤

- Checkpoint 保存版本、涵蓋的原始訊息數、SHA-256 前綴指紋與摘要。
  每次重建先驗證原始前綴；修改、截斷或內容能力改變後，失效摘要不會被重用。
- Checkpoint 與 usage 和完整 transcript 一起以同目錄 atomic replace 保存。
  舊 JSON 沒有 context 欄位仍可載入；寫入失敗時回復先前 context 並顯示錯誤。
- 新聊天、fork、編輯／Retry 截斷會清除原 context；重新載入自己的聊天可恢復。
- Stop／切換聊天會取消整個流程，包括摘要 HTTP 與正式回答，不會在背景偷偷續送。
  未完成（只有 EOF、沒有結束標記）、空白、過長、被截斷、僅複述先前助手回覆
  或出錯的摘要不會成為新 checkpoint。
- 若壓縮後仍超過預算或沒有減少上下文，停止正式回答請求，不以刪除舊資料硬湊。
- 只有結構化的 context overflow error code 才可觸發一次恢復；若已收到文字、
  reasoning 或工具 Activity，就不自動重送，避免重複工具副作用。
  401、429 與一般錯誤文字不會冒充 context overflow。

## 首版邊界

- 圖片、原始 PDF 與工具回合不作文字摘要或丟棄：可壓縮它們之前的文字，
  但若這些原始內容阻止安全縮減，就顯示錯誤並保留資料。
- Codex app-server 由其原生代理管理 context；不另外啟動具有 coding tools
  權限的摘要回合。OpenAI-compatible 與 ChatGPT subscription 使用本流程。
- 摘要會損失細節，無法保證所有任務無損；精確程式碼／附件應留在近期原文。
  尚未驗收多模型、所有附件組合、原生 GUI 的長時間真實聊天。
- 本次沿用 0.2.1 版本重建發行；不會自行重裝或啟動使用者的聊天 UI。

## 驗證方式

```powershell
cargo test --locked --lib chat::compaction::tests -- --nocapture
cargo test --locked --test auto_compact_status -- --nocapture
cargo test --locked --all-targets
```

自動測試使用隔離 synthetic transcript、file-only secrets 與 mock HTTP；不讀取
使用者的服務金鑰或送真實 API。`examples/auto_compact_probe.rs` 是先前經同意後
使用的 opt-in 真實模型效果實驗，不屬於正常測試或正式功能，執行需要 `--live`。
實測失敗與修正後結果均保留在核准的暫存目錄，不把單一文字情境當成全面驗收。

本輪最終 fmt／all-target check／diff check 通過，all-target tests：
**562 library、26 app、1 store-to-pipeline integration、15 importer passed；3 ignored**，
無 warning。包含反覆壓縮、停滯摘要 HTTP 取消、完整結束標記、存檔失敗回復、
重新載入、legacy JSON，以及遲到事件不污染新聊天等保護。
