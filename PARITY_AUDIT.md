# Swift → Rust 行為對照稽核（2026-10-28）

> 以下矩陣是本輪實作前的靜態快照。後續接線與驗證另記於
> [PORT_PROGRESS.md](PORT_PROGRESS.md)，不要把歷史缺口誤讀為最新執行狀態，
> 也不要把新紀錄誤讀為全部 Swift 函式已等價完成。

## 結論與範圍

**目前不是逐 function 完整移植，也不能稱為原版功能完整等價。**
先前工作以模組重寫、部分接線及 Rust 自身單元測試為主，未建立完整的
「原版函式 → Rust 實作 → 執行入口 → 等價驗證」矩陣。UI 截圖、編譯成功與
單元測試通過不能補足這個缺口。

- 原版：`D:\playground\popchat_tmp\Sources\PopChat`
- Rust：`D:\playground\popchat_windows\src`
- 此次為靜態稽核，不新增功能、不以本機模擬回覆當作真實 provider 驗證。
- 原版含 32 個 Swift 檔；`main.swift` 還含大量 smoke harness。
  函式數、檔案數或 Rust 測試數都**不是**完成率。
- 本文件先記錄有明確證據的重要差異；不是聲稱已列完全部 Swift 函式。

狀態定義：

| 狀態 | 意義 |
|---|---|
| 已接線 | 找得到實際應用程式入口；不代表所有邊界行為已證明等價 |
| 部分 | 有入口但有已知簡化或不同語義 |
| 僅核心 | Rust 有函式或測試，但使用者路徑未接通 |
| 缺失 | 沒有對應能力，或只有明確 stub |

## 指令、儲存、渲染與更新

| 原版函式／行為 | Rust 對應與證據 | 狀態／差異 |
|---|---|---|
| `PromptShortcut.expand`（`Chat/ShortcutStore.swift:11`） | `chat/store.rs::PromptShortcut::expand`、`chat/commands.rs::PromptShortcut::expand`；`ChatStore::send` 執行 slash 展開 | 已接線；兩套型別／實作仍需整併，不能只驗證其中一套 |
| `ShortcutStore.match`（同檔 `:48`） | `chat/store.rs::ShortcutStore::find`；composer 使用 commands 版本的唯讀鏡像 | 部分；Rust 使用 ASCII case-insensitive，而原版 Foundation case-insensitive 並非只處理 ASCII |
| `ShortcutStore.add/remove/persist`（同檔 `:52/:56/:60`） | `chat/store.rs::ShortcutStore::add/remove/persist` | 僅核心；設定頁沒有原版 Commands 編輯頁 |
| Commands 的 system prompt 編輯／還原（`SettingsView.swift:999–1015`） | `commands.rs::effective_system_prompt` 存在；`main.rs::send_options` 實際固定用 `ChatStore::DEFAULT_SYSTEM_PROMPT` | 缺失（使用者路徑）；不是可編輯 system prompt |
| `PromptHistory.record`（`Chat/PromptHistory.swift:22`） | `chat/store.rs::PromptHistory::record`；composer／send 呼叫 | 已接線；最近修正測試不得覆寫使用者 prompt history |
| `ConversationStore.save/load/resolveMessages/loadResolved`（`Chat/Conversations.swift:57–105`） | 活躍路徑是 `chat/store.rs::ConversationStore` 與 `ChatStore::persist/load_conversation` | 基本儲存／還原有接線；附件與跨版本 schema 必須另行驗證 |
| `materializeChildren/deleteMaterializingChildren`（同檔 `:114/:123`） | `chat/store.rs`、`persistence.rs` 各有實作 | 核心存在；須和 fork/delete 的真實 UI 入口分開驗收 |
| `fullTextIndex`（同檔 `:177`，排除 reasoning） | 活躍 `chat/store.rs::ConversationStore::full_text_index` 排除 reasoning；另一套 `persistence.rs::full_text_index` 卻包含 title/reasoning | 僅核心／重複實作不一致；不得混用兩套測試宣稱語義一致 |
| `excerpt/listRecent`（同檔 `:193/:208`） | `chat/store.rs::ConversationStore::excerpt/list_recent` 與 `persistence.rs` 各有實作 | 核心存在；歷史搜尋／瀏覽的可達性另列，不等同已做 HistoryPopover |
| `MarkdownRenderer.segments`（`MarkdownText.swift:30`） | `ui/panel.rs::split_math_segments` + `ui/markdown.rs::split_fences/parse_prose` | 部分；不是原版完整 segment 集合 |
| `.table`、`.pasteable`（`MarkdownText.swift:16/:20/:63/:120`） | `ui/markdown.rs:27–33` 明列不在子集；`FlowBlock` 沒有 Table/Pasteable | 缺失；system prompt 卻仍要求模型輸出 `<pasteable>` |
| `mathImage`（`MarkdownText.swift:479`，SwiftMath 排版） | `ui/math.rs::render_uncached` 驗證 MathML 後未使用其版面，`math_svg` 直接把原 LaTeX 放入 SVG `<text>` | **部分且明顯不等價**；不是分式／上標／矩陣的真實數學排版 |
| `highlightedCode`（`MarkdownText.swift:456`） | `ui/markdown.rs::highlight_line/show_code_block` | 部分；手寫 Rust/Python/Swift 子集，不是原版整個 highlighter 能力 |
| `SelectableText`（`MarkdownText.swift:548` 起） | 多個 `egui::Label`、獨立 code copy 按鈕 | 未達原版跨 block 的選取、量測快取與搜尋定位語義 |
| `TextReveal.eased/ramp/groups`、`RevealFade.paint`（`Reveal.swift:37/:52/:71/:84`） | 目前 renderer 無對等 fade 實作 | 缺失；ChatStore 內 pacing helper 的存在不能算 glyph/sentence fade 已完成 |
| `UpdateChecker.checkIfDue/check`（`UpdateChecker.swift:65/:75`） | `update.rs::check_if_due/check_for_update` | 僅核心；main／設定／tray 沒有呼叫，沒有實際更新通知或手動檢查入口 |
| `fetchLatest/decodeRelease/isNewer/components`（同檔 `:105/:134/:148/:163`） | `update.rs::fetch_latest/decode_release/is_newer/version_components` | 核心可測；不代表應用已排程執行 |
| 原版不自動下載／替換執行檔（`UpdateChecker.swift:3–11`） | `apply_self_update` 是 stub | 不應把「自動替換未實作」錯列為原版缺口；真正缺的是檢查與通知接線 |
| `FirstRunHint.show/dismiss`（`FirstRunHint.swift:28/:45`） | 無對應 tray 初次引導 | 缺失；空態顯示 Alt+Down 不是等價的首次啟動引導 |

## 實際聊天、工具與附件路徑

| 原版函式／行為 | Rust 對應與實際入口 | 狀態／差異 |
|---|---|---|
| `ChatStore.send`（`Chat/ChatStore.swift:198–228`） | `chat/store.rs:589`；`main.rs:809–830` 實際呼叫 | 已接線：基本文字、slash 展開、錯誤列。不能據此推論所有 provider 與附件可用 |
| `OpenAIChatClient.streamOneRound`（`Chat/OpenAIChatClient.swift:369–424`） | `chat/client.rs:642–714`，由 `main.rs:835–850` 轉送事件 | 已接線：OpenAI-compatible Chat Completions 的基本請求與 SSE |
| `decodeDelta/consume stream`（同檔 `:354–366/:431–454`） | `chat/client.rs:605–638/:721–769` | 部分：`[DONE]` 只跳出內層行處理，還會等外層 HTTP stream；與原版立即結束不同 |
| 串流事件／reasoning／waiting/activity/error | `Chat/ChatStore.swift:336–429` → `chat/store.rs:763–878`、`main.rs::poll_stream/show_list` | 已接線：狀態更新與顯示；不是逐字／逐句 drain 動畫 |
| `stop`（`Chat/ChatStore.swift:705–712`） | `main.rs:855–860` abort 轉送 task；`client.rs:378–380` 另有 detached worker | 部分：UI 停止，但尚未保證立即取消正在等待 headers／下一個 chunk 的 HTTP 請求 |
| `WebToolExecutor.execute/search/fetchURL`（`Chat/WebTools.swift:91–249`） | 真實實作在 `chat/web.rs:84–235`；實際 Chat Completions loop 使用的是 `chat/client.rs:269–274` 的同名 stub executor | **僅核心／實際路徑不可用**：工具呼叫回傳「not available in this build yet」，並未到 `web.rs` 的搜尋／抓網頁程式 |
| `run` 工具多輪對話（`Chat/OpenAIChatClient.swift:197–269`） | `chat/client.rs:398–488` 有多輪迴圈；`web.rs::run_with_tools` 是另一套、沒有 production caller | 部分：迴圈已接，但實際 local tools 是 stub；fake driver 的測試不能代表網搜已通 |
| `resolveWebAccess`（`Chat/ChatStore.swift:668–703`） | `chat/store.rs:335–372` 已由 send 呼叫；`main.rs:801` 固定 `search_engine_key: None` | 部分：toggle／引擎名稱有接；Brave/Tavily 金鑰設定與實際使用未接通 |
| 檔案選擇／圖片貼上／拖放／附件 chips／純附件送出（`ComposerView.swift:16–86/:675–782`） | `main.rs:815` 空文字直接返回；`:821` 每次都傳 `Vec::new()`；composer payload 只有 `String` | **缺失（使用者路徑）**：不是已支援附件 |
| `AttachmentLoader.load` 至 transcript／wire（`Chat/Attachment.swift:8–54`） | loader 的 `attach::Attachment` 與活躍 store 的 `store::Attachment` 型別不同，沒有接線轉換 | 缺失：loader、store、獨立 persistence 還有重複資料模型 |
| `loadPDF`（同檔 `:143–189`） | `chat/attach.rs:380–381/:452–455` | 缺失：所有 PDF 都被明確拒絕；測試驗證的也是 stub error |
| `loadWordDocument`（同檔 `:194–210`） | `chat/attach.rs:383–386` | 缺失：DOCX/DOC/RTF/ODT 明確不支援 |
| `loadImage`（同檔 `:94–138`） | `chat/attach.rs:397–445` | 僅核心，且語義不同：1536 vs 2048 長邊、PNG 保留 vs 原版 JPEG，HEIC/HEIF 路由缺失 |
| 試算表／文字讀取（同檔 `:215–304`） | `chat/attach.rs:462–580` | 僅核心：有抽取／截斷等邏輯，但實際 UI 無載入入口 |
| `recordManualCapabilityException` + 錯誤列動作（`Chat/ChatStore.swift:124–130`、`ChatView.swift:332–357`） | `chat/store.rs:571` 有函式，主畫面錯誤列只有文字 | 僅核心：沒有使用者可操作的手動標記入口 |

## Provider、ChatGPT 與 Codex

| 原版函式／行為 | Rust 對應與實際入口 | 狀態／差異 |
|---|---|---|
| `ProviderStore.init/persistJSON`（`Chat/ProviderStore.swift:245/:1029`） | `providers.rs::load/try_save`，啟動與 Save 呼叫 | 已接線：基本 provider/model 儲存與讀取 |
| `addCustom/addTemplate/uniqueName/remove`（同檔 `:795–832`） | `providers.rs:1273–1345` 有方法 | 僅核心：設定頁只能選既有服務，無新增／移除服務入口 |
| `configuredProviders/hasSetupEvidence`（同檔 `:465/:478`） | `providers.rs:931/:945` | 僅核心：沒有原版 configured-provider rail/switcher 消費這些結果 |
| `select` + catalog-only Settings（同檔 `:638`、`SettingsView.swift:64–69`） | `main.rs::save_settings` 直接變更 live `selected_id` | 部分／不同語義：原版在 pill 切換 live model；設定頁只管理 catalog，Rust 把這兩者混在一起 |
| `fetchModels/decodeModelList`（同檔 `:916/:874`） | Settings 呼叫 `fetch_models_list` 後 `.map(|(models, _)| models)` | 部分：真的會抓模型名稱，但把 capabilities metadata 丟掉；完整 `fetch_models_for` 沒有 production caller |
| `probeOllamaCapabilities`（同檔 `:986`） | `providers.rs:426` | 僅核心：只由未接線的完整 store-fetch 路徑呼叫 |
| reasoning effort 選擇（同檔 `:508–533`、`ProviderSwitcher.swift:481`） | `providers.rs::supported_efforts/remembered_effort/set_effort` | 部分：預設值能進 config；沒有實際 effort 選擇 UI |
| `ChatGPTAuth.signIn/signOut`（`Chat/ChatGPTAuth.swift:133/:123`） | `chatgpt_auth.rs:218/:228` | **僅核心**：無主程式 caller，設定頁也明寫沒有登入／登出功能 |
| PKCE／callback／exchange／JWT extraction（同檔 `:169–347/:428`） | `chatgpt_auth.rs::wait_for_callback/callback_handler/exchange_code/build_tokens` | 僅核心；還有 browser 先開、listener 後綁定的先後差異 |
| `validCredentials/refreshedCredentials/refresh`（同檔 `:361–382`） | `chatgpt_auth.rs:406–431` | 僅核心：沒有實際請求 client 呼叫；token store 也未整合到應用的 SecretStore |
| `CodexResponsesClient.run/streamOneRound/consumeStream`（`Chat/CodexResponsesClient.swift:35/:198/:281`） | 無對應的完整 Responses HTTP/SSE transport | **缺失**：不是 `codex.rs` 的本機 app-server 就能代替 |
| ChatGPT 執行路由（`Chat/ChatStore.swift:324–331`） | `main.rs:802` 固定 `chatgpt_signed_in: false`；`store.rs:662` 因而先拒絕 send | **不可用（使用者路徑）**：不應宣稱已支援 ChatGPT 訂閱聊天 |
| ChatGPT/Codex delegated backend 註冊 | `client.rs:308` 有 `set_delegated_backend`，但全 repo 無呼叫／subscription implementation | **未接線**：`main::start_stream` 有呼叫 `client::run` 路由器，問題是路由器後面的 backend 沒有註冊；Codex 會得到明確的 no-backend error |
| Codex executable／JSONL Session（`Chat/CodexAppServerClient.swift:56/:1026/:1104`） | `codex.rs::resolve_executable/AppServerSession::spawn/request` | 僅核心：沒有 main 的 CodexClient caller 或 event adapter |
| Codex `inspectBlocking` 模型／effort catalog（同檔 `:269–294`） | `codex.rs::check_ready` 只初始化／讀 account | 部分且未接線：缺 `model/list` 對應及 producer → `CodexInspection` 的接線 |
| Codex `runTurn/splitHistory/responseItem/turnInput`（同檔 `:371/:739/:761/:786`） | `codex.rs::stream_turn/drive_turn/split_history/response_item` | 部分且未接線：公開入口只收 prompt/model，實際 prior history 為空、只有文字 input |
| Codex native search、取消／timeout、事件回傳 | `codex.rs` 有部分事件與限制邏輯，但使用另一套 `codex::ChatStreamEvent`；主程式收 `client::ChatStreamEvent` | 部分且未接線：無轉換 adapter，web toggle 也沒有被帶入 Codex 設定 |

## UI 與生命週期

| 原版入口／行為 | Rust 實際狀態 | 分類 |
|---|---|---|
| HistoryPopover 開啟、方向鍵選擇、刪除（`ChatView.swift:210–226`、`HistoryPopover.swift:203–260`） | `ChatStore::load_conversation/delete_conversation` 存在，但頂欄無歷史按鈕，無 popover caller | 僅核心 |
| 跨對話搜尋（`HistoryPopover.swift:319–347`） | 有 `searchable_text/index/excerpt`，無搜尋 UI | 僅核心 |
| 當前對話 Find、上／下一個命中、highlight（`ChatView.swift:465–585`） | 無 Ctrl+F/G、find bar、命中數或定位 | 缺失 |
| 整則回覆 Copy／Fork（`ChatView.swift:820–858`） | 有 code block copy；無整則回覆 copy/fork 動作 | 僅核心／部分 |
| Retry（`ChatView.swift:354/:927–933`） | `retry_last_turn` 只回傳 payload，無重新送出的 caller | 僅核心 |
| Edit prompt（`ChatView.swift:454–458`） | 有 `take_last_user_message_for_editing` 和 `Composer::set_text`，沒有人把它們接起來 | 僅核心 |
| New chat（`ChatView.swift:227–237`） | 有 `new_chat`，無按鈕／Ctrl+N caller | 僅核心 |
| Model/provider/effort switcher（`ProviderSwitcher.swift:86–131`） | 帶 chevron 的模型區實際只切換 Settings，不是原版 switcher | 部分；不能把外觀看似下拉當作已完成原功能 |
| Enter／Shift+Enter、slash 完成、prompt recall | 實際接線存在；IMEs 的 marked-text guards 與原版共用 native text view 的 caret/undo 契約未建立 | 基本文字／補全／回憶已接線，IME／編輯器等價性部分 |
| Appearance/accent（`SettingsView.swift:170–328`） | 有主題與 accent；無原版 bubble-style、glass/tint、streaming-mode 設定 | 部分 |
| launch at login（`SettingsView.swift:139–159`） | 沒有 Windows 開機啟動註冊／取消入口 | 缺失 |
| 使用者錄製快捷鍵（`SettingsView.swift:1110–1119`） | 固定 Alt+Down；依本次 Windows 使用需求的明確改動，不同於原版 recorder | 已知平台／需求差異，不能宣稱 recorder 已移植 |
| 非 activating panel／返回先前 app（`FloatingPanel.swift:11–21`） | Rust 用 `SetForegroundWindow`；未實現原版完整 nonactivating／previous-app focus semantics | 部分 |
| Resize／position persistence／monitor validation／動態成長（`PanelController.swift:253–436`） | 大小可存，但 native resize 被關閉，位置只在 runtime controller 記住，`last_x/last_y` 不回復，無等價螢幕交集檢查／動畫 | 部分 |
| Tray／首次啟動／login-aware 啟動／reopen（`AppDelegate.swift:55–172`） | tray toggle/settings/quit 有接；1×1 placeholder tray icon，無 first-run／login-aware/reopen 等價路徑 | 部分 |

## 應先補的順序（不是已獲准或已完成的實作清單）

1. **核心接線**：真實 web executor；ChatGPT auth + Responses backend；Codex backend adapter、history/catalog。
2. **對話基本操作**：New / History / Load / Delete / Retry / Edit / Fork，逐一接 UI 並測試。
3. **附件整併**：只留一條 loader → draft → store → wire 路徑，再接 picker/paste/drop，最後補 PDF/Word。
4. **原版輸出語義**：table/pasteable、真正公式排版、逐字／逐句 reveal 與 Stop 的網路取消。
5. **設定與生命週期**：Commands/system prompt、provider CRUD/model/effort switcher、search credentials、updates/startup。

## 驗收原則（後續必須採用）

1. 每項必須對到原版的函式或 SwiftUI action，而不是自行想像功能。
2. 檢查 Rust 的實作、實際 caller、狀態轉移、持久化與錯誤／取消路徑。
3. 只有 library 函式或測試，標為「僅核心」，不得標「完成」。
4. 單元測試先用原版相同輸入／預期值；可抽離的演算法使用 differential/golden cases。
5. UI 操作另外做端到端驗證；本機 mock SSE 只證明那條測試路徑可跑。
6. 平台差異（AppKit → Windows/egui）可換實作，但必須記錄行為替代，不能用它掩蓋缺失。
7. 沒有逐項驗收證據以前，不報功能完成率或「完整移植」。
