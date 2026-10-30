# 原版行為移植：本輪接線紀錄

2026-10-30 靜態來源更新。這不是完整移植宣告，也不是完成率。
`PARITY_AUDIT.md` 保留先前快照；目前完整函式清冊與具體缺口以 `FUNCTION_PARITY.md` 為準。
清冊的函式對照來自靜態來源審查；parent 的最新測試、隔離原生操作與截圖另記於文末。
下表描述實作及測試覆蓋邊界，不能單憑 caller 存在推定真實帳號／完整原生驗收。

**目前邊界：** 唯一 UI 為使用者活躍的 **PID 26008**（canonical x64 release，頂部同列／訊息 icon／scroll inset／無底部提示列，未釘選），已查看最後 production build 截圖。使用者現在選 Dark、90% 背景、CursorScreenCenter、英文 TT Norms Pro Normal／中文使用者目錄的 Swei Gothic CJK TC DemiLight；不能用舊 checkpoint 恢復成 Light/default 字體。使用者允許直接重啟；仍須 WM_CLOSE／等待退出／確認零實例後替換，不送出聊天或自行 Save API。

## Swift → Rust → 應用入口

| 原版函式／動作 | 本輪 Rust 路徑 | 驗證邊界 |
|---|---|---|
| `OpenAIChatClient.run/runLoop/streamOneRound/decodeDelta` | `chat/client.rs::run/run_loop/stream_one_round/decode_delta`，由 `main::start_stream` 呼叫 | 真實 HTTP/SSE 本機 server；DONE 前結束連線、UTF-8/CRLF/最後一行、停滯連線取消 |
| `WebToolExecutor.execute/search/fetchURL` | `client::WebToolExecutor` 轉發到 `web::WebToolExecutor` | 實際 `client::run` 工具呼叫 → HTTP fetch → tool result → 第二次模型請求，不再是 stub |
| `CodexAppServerClient.inspectBlocking` | `codex::inspect`，設定頁檢查動作 | 假 app-server 執行檔跑完整 JSONL initialize/account/model-list，非真實 Codex 帳號 |
| `runTurn/splitHistory/responseItem/turnInput` | `codex::CodexBackend` 與 `backends::DelegatedBackend` | 正式 adapter 的歷史、圖片、effort、native search、事件與子行程終止測試 |
| `ChatGPTAuth.signIn/withCallbackServer/validCredentials/refresh` | `chatgpt_auth` 與 NativeTokenStore；設定頁登入／取消／登出 action | PKCE 與 loopback／憑證儲存測試；真實瀏覽器登入需要使用者帳號，尚未驗證 |
| `CodexResponsesClient.run/streamOneRound/consumeStream` | `responses::ResponsesBackend`，由 delegated router 分派 | 獨立於本機 Codex transport；以本機 HTTP/SSE 驗證，不能宣稱訂閱服務實測通過 |
| `AttachmentLoader.load/loadPDF/loadWordDocument/loadImage/loadSpreadsheet` | `attach::load_store_attachment` → 活躍 `store::Attachment` → `wire_content_for_message` | 真實檔案 fixtures；PDF 原檔與抽取文字同存、DOC/DOCX/ODT/RTF/RTFD、表格、2048 JPEG |
| `newChat/loadConversation/deleteConversation/retryLastTurn/takeLastUserMessageForEditing/fork` | `main` 的實際動作呼叫 `ChatStore`；非僅保留方法定義 | 新增對話、歷史搜尋／載入／刪除、Retry/Edit/Fork/Copy 與原附件 payload 接線 |
| `ChatView.toggleFind/stepMatch` | 應用內 Find 狀態、命中定位、Ctrl+F/G | 當前對話與跨對話搜尋分開，搜尋不把隱藏 reasoning 當作正文 |
| `ShortcutStore` + Commands 設定 | `SettingsUi::commands_section` → `save_system_prompt_only/ShortcutStore::save_entries` → `CommandsChanged` → `main::settings_actions` 更新活躍 shortcuts | system prompt（含明確空值）、shortcut 新增／刪除／編輯獨立即時存檔；保留未知 JSON 欄位、不順帶提交 API 草稿，失敗可重試。活躍 slash 執行與補完皆採 Unicode `to_lowercase`；不再是 ASCII-only，但不宣稱完全等同 Foundation folding |
| 一般偏好／搜尋金鑰／Codex 路徑 | `SettingsUi::show → persist_general_preferences`；路徑輸入／原生 picker → `CodexPathChanged → main::settings_actions/persist_codex_path` | 一般偏好、原生儲存搜尋金鑰、Commands、Codex 路徑獨立即時保存；路徑改動更新 backend 並使 inspection 失效。只有 API endpoint/key/model catalog 草稿仍採明確 Save |
| `fetchModels/decodeModelList/probeOllamaCapabilities` | 設定的 `fetch_catalog` 保留 capabilities 並探測 Ollama | 正式設定 fetch 路徑的 wiremock 測試；UUID/generation 保護仍保留 |
| `ChatStore.startDrain/typewriterStep/sentenceCommitLength` | `ChatStore::tick_reveal/display_text`，UI 時鐘驅動 | 逐字／逐句、完成後加速 tail、Stop flush、非單調 snapshot、Swift Character grapheme；完整 Copy 仍讀 full_text |
| `MarkdownRenderer.segments` + table/pasteable／SelectableText | `main::show_list → MathTextures::show_assistant_message → panel::split_math_segments` → `markdown` 各 renderer | 原版順序／未閉合案例 golden；真實 headless 拖選／Copy 涵蓋 prose 段落、標題、格式、連結。code／quote／各 table cell／pasteable 分開選取是原版本來的 island 邊界，不是全面跨 block 選取缺口；bitmap 物件／rich clipboard 仍有差異 |
| `AssistantMessageView.fade/RevealFade.paint` | `ChatStore::tick_reveal/reveal_for` → `main::show_list → ui::reveal::paint_reveal`，正文 renderer 後、Find 前 | 最後 prose/code segment 的 glyph grapheme alpha-band／sentence fade 已有實際呼叫，限定正文 shape range、不影響 reasoning／控制項；公式 bitmap mesh 仍保持實色，streaming caret 未接線 |
| `AccentPicker` HSB／Hex／Reset | `SettingsUi::show → appearance_section → accent::AccentPicker::show` → 一般偏好 autosave | 有狀態的 rainbow custom swatch／popup／三條 gradient slider；開啟 unset 不寫入、gray 保留 hue、slider 到 black 保留 saturation、不自我 reseed、量化同色去重；Hex Enter 驗證六位、拒絕錯誤／換行並回復現值，Reset 清 custom／回 Blue；原生視覺／accessibility 尚未驗收 |
| `Theme.bubbleFill/bubbleForeground` | Settings `appearance_section` → `BubbleStyle`；`main::show_list` user row → `AccentPalette::for_bubble_style → bubble_frame/user_text` | accentTint 為 light 15%／dark 19%，accentFill 使用 accent 實色及 CIE L* 對比字；`quietGray`／未知舊值 fallback tint。設定獨立即時保存，並非只有 enum／測試沒有 UI caller |
| `WaitingIndicator/ThinkingScroller/ElapsedLabel` | `main::start_stream → WaitingView::start_turn`；`show_list → WaitingView::show`（streaming 且正文未到） | 固定 3／12 行可選取捲動、只 strip 新 suffix 的 `*`／backtick／`#`（保留 `_`）、replacement reset、append 前 8pt bottom-follow 判斷、穩定每 turn 時鐘／8 秒才顯示計時、1.6 秒 pulse／頂端 22% fade；非 NSTextStorage 增量排版，效能／OS Reduce Motion 限制見下 |
| `ReasoningDisclosure/attributedReasoning` | `main::show_list → reasoning::show_disclosure → markdown::show_prose` | 每則訊息獨立、預設收合；settled reasoning 有格式化／可選取 Markdown，維持一般 renderer 字級、僅改 secondary 色並有左側線；非縮小字級純文字替代，亦不宣稱原生 attributed object／動畫完全一致 |
| `mathImage` | `latex-rust` AST/OpenType MATH → STIX 字形輪廓 → resvg raster | 分式／根號／上下標／矩陣／aligned 結構與像素測試；不再是 LaTeX 原文成像；不宣稱完全相容 SwiftMath |
| `UpdateChecker.checkIfDue/check` | 應用持有的 scheduler + 設定手動檢查／發行頁連結 | 啟動延後 5 秒、24h、關閉自動檢查、背景錯誤靜默／手動錯誤顯示；不自動下載或替換 exe |
| `SMAppService` 登入啟動 | `startup.rs` 目前使用者 Run entry + `--login-item` | 命令引號／旗標測試；不在自動測試改動真實登入項目 |
| 首次啟動／single instance／tray 引導 | free `main` 在 eframe 前取得並保留 `InstanceGuard`；hidden-capable visibility driver 收 `Show/Settings`；`startup` launch policy；app `update → PopchatTray::show_first_run_hint` | 首次 non-login 顯示、login／重複啟動隱藏、Settings 優先；reopen 是 show 不是 toggle；350ms 引導及 dismiss 已接線，為自己的 tray HWND／icon ID Windows shell balloon，不是 macOS anchored popover。原生多行程／shell lifetime 未驗收 |
| `PanelController` geometry／resize | `PanelGeometry/ViewportSizing` → `main::new/update_window_geometry/show_card`；原生 resizable viewport／resize rim／save debounce | work area／位置還原／明確 user-resize handshake、compact 垂直鎖定與偏好保存已有應用 caller；程式化／DPI／mode resize 不應蓋掉使用者偏好。自然高度／動畫與原生 mixed-DPI matrix 仍不同或未驗收 |
| 底部 footer 裁切 | `main::show_card → chat_layout::bottom_panel/show_body` | 實測排版高度取代固定 92pt reserve；Find／讀取附件／composer／footer 先保留，transcript 限定剩餘區域；底部超高可捲動、不攔截文字拖選。604×344／644 有限原生截圖已讀取，不是 native compact-window 自然高度全等 |
| 英文／中文獨立字型 | Settings `appearance_section → fonts::show_font_preferences`；`main::new/update → apply_font_preferences` | 分開挑已安裝且 coverage 檢查通過的 curated 字型；英文 proportional 選擇不替換 code font，CJK fallback 供 proportional／monospace；錯誤保留最後可用組合。Georgia／標楷體獨立切換與 autosave 已原生確認；不是任意字體匯入或全字形矩陣 |
| 可設定全域熱鍵 | Settings `hotkey_section` draft／Apply → `ChangeHotkey → main::settings_actions → HotkeyManager::set_binding → SettingsUi::commit_hotkey_binding` | **Alt+Down 改為預設而非固定**；Alt+Space 仍拒絕。先註冊新鍵再撤舊鍵，OS 失敗保留舊鍵／已存設定；成功才 commit，存檔失敗另外顯示，general autosave 不帶入未 Apply draft。原生換鍵／重啟／受控 OS 衝突已驗證 |
| 背景透明度／拖曳／預設位置 | Settings `appearance_section → window_prefs::show_preferences`；`main::show_card → surface::card_frame_with_opacity/drag_window`，`header_row` brand／gap drag；`main::new/update_window_geometry → window_prefs::placement_rect` | 35–100% 僅改 backdrop／border／shadow，文字與控制項不套整層 alpha；header／top band 可拖、不覆蓋 transcript／composer／按鈕。六種位置僅 startup／新 SHOW 套用，monitor-only change 只驗證／clamp 目前位置；獨立 autosave。原生確認 55%／兩個 drag target／Fixed／Remember；其他四模式與混合 DPI 未驗收 |

## 仍需保留的差異／缺口

- 真實 provider API、ChatGPT 訂閱與使用者 Codex 安裝／帳號：沒有登入憑證，不算實測通過。
- Reasoning 雖只處理新增 suffix 的 emphasis stripping，累積字串 equality／prefix 比較與 galley clone／hash／layout 仍是 O(n)，成長中的 turn 仍可能有平方級總成本；固定 viewport 不等於 NSTextStorage 增量排版效能。pulse 是有上限的 30Hz UI repaint，不是 CA 獨立動畫；計時也未證明只更新 leaf。
- `animation_time == 0` 可停 reasoning pulse，但沒有自動讀取／橋接 Windows Reduce Motion；原生 accessibility／字型 metrics／長 thinking trace CPU 與 disclosure／expand 動畫未驗收。
- prose 段落／格式／連結共享選取、原版刻意獨立的各 island 已接線。剩下的是公式 bitmap 不作為可選取物件／rich clipboard attachment 複製，文字 fade 也不處理 math mesh；不能再概括為所有跨 block／連結選取都缺失。
- 語法高亮仍是手寫 lexer／theme／cache；原版 `highlightedCode` 呼叫 Splash 預設 Swift highlighter、不傳 fence language，沒有「原版完整多語言 catalog」這項要求。精確 Swift token／色彩／行距／快取仍需比較。
- HEIC/HEIF 解碼、完整 SwiftMath 相容性及任意 PDF／DOC 等文件抽取相容性仍未保證。CF_HDROP clipboard routing、path／byte drop 與 hover feedback 已接線且有下方有限原生證據；不等於 Windows FileDescriptor／virtual-file promise、raw OS image-drop 轉換矩陣或 AppKit panel-wide format precedence 完全移植。
- AppKit nonactivating 並未等價移植；Win32 foreground／還原前一視窗是 best effort。tray balloon／並行 processes／Settings forwarding／resize／monitor／DPI／IME／封裝資源與原生焦點矩陣均需另外驗收，不是尚無 caller。
- 底部 composer／footer stack 已改實測與有界排版，非固定 92pt reserve；但 native compact-window／soft-wrap／長 notice 自然高度仍採估算，unchanged-row／長 transcript 排版成本未證明等價。streaming caret、restore group reveal、row／show-hide／growth 動畫、transcript 頂部 mask、新訊息跳轉 pill 仍缺或部分實作；reasoning 頂部 fade 不能當 transcript mask 完成。
- provider→model→effort 三欄 inert preview／鍵盤 Enter commit／setup CTA／Settings readiness 點仍未等價；Settings 是 embedded surface，不是保留的獨立浮動視窗，blur 語意也不同。
- glass／panel tint／Reduce Transparency／專用 bubble preview、composer globe、header wheel forwarding、tray update item、history cascade／flash／relative labels、原版 context menu、附件總 payload warning、Settings capability/source-label UI 等仍有明確缺口。
- Unicode lowercasing 不保證 Foundation locale／case folding 全等；completion 拒絕所有 whitespace、原版只檢查 literal space 的邊界仍在。Find count／glyph／table 對齊、Codex partial-line watchdog／共享 refresh await 契約、跨版本資料 migration 與 silent conversation save failure 仍需比較。
- 完整清冊保留所有 PARTIAL／CORE_ONLY／MISSING 明細，本摘要不刪除其他非本輪範圍的缺口。熱鍵現為可設定、預設 Alt+Down，Settings 文字 draft／Apply 是 native recorder 的平台替代；不再沿用已被使用者覆蓋的「固定 Alt+Down」限制。六項新需求已有實際 caller，仍不可把舊原生截圖當成六項驗收。

## 清冊核對

`FUNCTION_PARITY.md` 保留 **32 個 Swift 檔、513 個 explicit definitions**：非 `main.swift` 450、`main.swift` harness helpers 63。
每筆原版宣告行號／overload／operator row 均保留；UI properties／inline actions／top-level smoke 額外列示，不混入函式總數。
Markdown fixture 中的 `main.swift:1016 func fib` 不計入，真正的 `MessageRow.==` 計入。
清冊覆蓋不等於行為全部驗收，不提供完成百分比。

## 最終整合驗證

### 完整已安裝字體／米黃淺色

- `font_catalog.rs` 背景讀取系統／LocalAppData／Roaming 使用者字體與 HKLM/HKCU Fonts 登錄的外部字體；TTF／OTF／TTC／OTC 字面各自持久化 path/index，支援中英名稱搜尋、virtualized rows、重新掃描。保留舊 filename 偏好；不安裝／修改 OS 字體。僅保留 metadata；無法讀取、僅點陣或超過 64 MiB 的檔案會略過並在 hover 說明，非任意外部匯入。
- `font_cmap.rs` 在記憶體隔離英／中文 cmap，避免含漢字的英文字體覆蓋所選中文；任選缺字字體仍保留完整 CJK／原始 Latin／emoji 備援。沒有改磁碟上的字體；IVS／完整複雜文字 shaping 不宣稱等價。程式碼仍使用等寬字體。
- `theme.rs` 集中米黃 token：panel `#F5EDDD`、control `#EDE1CA`、raised `#FAF2E3`；設定、composer、popup、Markdown／code neutral surface 一起套用。這是使用者明確覆蓋原淺色設計；深色、accent、透明 clear、實際圖片／公式 pixels 不改。
- fmt／all-target check／full suite 曾通過 **429 library passed、2 ignored、23 binary passed**，x64 button-review release 成功，單一替換 PID 34176 → 44856。後續搜尋副檔名干擾修正及新回報的正文問題尚待最後 full-suite/build/native 重驗，不把上述數字當最後 source。
- 原生 `installed-fonts-beige-new.png`／`installed-fonts-beige-appearance-selected.png` 親自確認米黃設定／輸入框、337 個已安裝字體。`installed-fonts-beige-catalog.png` 拍到使用者自己正在選字體／輸入 TT，不冒充助理選取測試，也不覆寫該選擇。已停止原生輸入；搜尋 TT 誤匹配所有 `.ttf` 已改搜尋 filename stem，待下次 build。
- 使用者目前偏好：Light、CursorScreenCenter；自選 TT Norms Pro Normal（英文）／Yu Gothic UI TTC face 1（中文），不能依舊 checkpoint 恢復成 default/Remember。新增這對字體的 proportional／monospace、12/13/15/19pt glyph UV＋實際 atlas alpha 測試通過，字形本身有墨跡。正文空白仍須追查長 HTML／code 的可視範圍與 layout，不將 raster test 當 bug 已修。
- 最新對話僅讀取 role／長度 metadata 確認已有實際回覆（assistant 1384 characters），未再送出請求；其 user HTML 有 208-character 行。PowerShell 第一次未指定 UTF-8 的 metadata parse 失敗並輸出錯誤上下文，**不能當原始 JSON／app Unicode 資料損壞證據**；之後改顯式 UTF-8、捕捉錯誤而不輸出內容。先補 `main::show_list` RTL user label 的明確 `.wrap()`，仍待完整 renderer 的可視 clip／mesh regression 與原生驗證，不能先宣稱根因已排除。

#### 正文空白：排版重現與修正

- 已確認 RTL user bubble 的預設 `Label::Extend` 是橫向溢位來源。隔離的 production-layout／Markdown fixture（非真實聊天量測）重現：483pt viewport 的 content width 膨脹到約 7145pt、user text 左端約 −6611pt，底部 assistant glyph triangles 全在 clip 外。
- 正式 `main::show_list` User caller 改 `Label::new(...).wrap()`。相同 fixture 的 content width 回到 viewport；正文、語言標籤、兩個 code fence、Copy 都有非透明 tessellated glyph triangles 與有效 clip 交集，不只是檢查字串／galley 存在。
- 新 `chat_layout::tests::long_user_html_must_wrap_to_keep_transcript_glyph_meshes_visible` 比較 legacy/fixed、normal／實際 TT Norms＋Yu Gothic UI 字體及切換、553×594／520×344／604×644、1/4-line composer、top/bottom scroll；3 個 chat-layout tests 通過。最後 full-suite/build／原有對話原生重驗待補；不需要重送 API 問題。
- 最後 fmt、all-target check、full suite 通過，無 warning：**431 library passed、2 ignored、23 binary passed**；canonical release build 成功。先關閉唯一 PID 44856、等待退出並確認零實例後，不帶 review pin 正常啟動為 PID 39864，保留 Light／CursorScreenCenter／使用者選的 TT Norms＋Yu Gothic UI；確認唯一程序。
- 確認當時已存的 Alt+Down 後只叫出一次，親自讀取 **`transcript-wrap-fixed-visible.png`**：原有對話的兩個 CSS code block、`css` 標籤、段落、Copy／訊息操作與 footer 都恢復可見，並保留米黃面板與 code 背景。沒有重新 Send／Retry／Copy、沒有改 clipboard、沒有按 Save API；pin 圖示為未釘選。這是原有回覆的修正後畫面，不是假 server／新測試訊息。

### 新聊天／歷史圖示按鈕

- 依最新使用者要求，`controls::chat_navigation` 改呼叫與 Settings 相同的 `icons::button`：28×28 hit target、透明待機／圓形 hover、history active accent、可辨識的 NewChat／History vector paths。保留原 `ChatNavigation` action caller、Ctrl+N／Ctrl+Y tooltip 與 button widget info，不把新增對話動作拿來操作使用者現有聊天。
- 增補 icon 尺寸清冊與兩個純 UI callback click 測試；沒有恢復常駐 Find。完整測試初次通過 **432 library passed、2 ignored、23 binary passed**；僅新增 test closure 有 unused-mut warning，已移除後重新執行最後 fmt／check／full suite（結果另補）。release button-review build 成功；warning cleanup 僅在 cfg(test) 中，不改 production rendering。
- 依序關閉 PID 39864、等待退出且確認零實例後，用相同 profile、不帶 review pin 正常啟動 PID 16020。只以確認的 Alt+Down 叫出一次，親自讀取 **`navigation-icons-visible.png`**：NewChat bubble-plus／History clock 與 Settings gear 同為小型線條 icon，保留原有正文／CSS code／米黃。沒有按 NewChat／Copy／Send 等會更動草稿或對話的入口；實際兩個 action callbacks 由無 profile 的 UI 測試驗證，未假稱原生點擊。截圖時使用者正在輸入草稿，保留原樣，不再操作視窗。

### 同列工具列／訊息 icon／左緣安全內距／移除底欄

- 頂部 `main::header_row → controls::header_actions_width/chat_navigation_inline` 依設定頁／聊天頁預留 3／5 個圖示位置，模型選單寬度隨之縮小；NewChat／History／Pin／Settings／Close 同列，不再額外畫第二排。
- `main::show_list → controls::message_actions_inline` 將 Copy／Fork／Retry／Edit 改為同一 `icons::button`；複製成功改 Check icon，保留 1.2s feedback、streaming Fork disabled、僅最後可重試列顯示 Retry/Edit，以及原有真正 action caller。UI 測試只檢查 callback，不操作 clipboard／聊天。
- `chat_layout::transcript_contents` 的 6pt 左右／2pt 上下內距由正式 `show_list` ScrollArea content caller 使用，不放大 clip；讓 frame stroke、negative glyph bearing、toolbar hover 圓形遠離硬裁切邊緣。新增 left-frame／first-glyph／circle 幾何檢查，不能在跑完與原生確認前宣稱左緣已完全驗收。
- 依使用者最新要求移除底部 Enter／Ctrl+V／拖入檔案／全域熱鍵提示整列與其 7pt gap；保留 composer、快捷鍵 handlers、paste/drop。移除只供 footer 使用的 app `hotkey_label` 副本，不動 HotkeyManager／Settings 的真實 binding 狀態。384 組 composer-layout fixture 同步改為無 footer，仍檢查自然高度／overflow／clip，不再要求不存在的 footer。
- 初次完整測試僅新 edge test 的「top scroll 也必須看得到 toolbar」失敗：fixture 的 toolbar 位於 340pt viewport 下方，正確被 cull。已改為 top／bottom 都檢查 code 邊界，bottom 才要求實際 toolbar paint，並遞迴檢查 shape groups；沒有放寬左右安全內距幾何條件。最後無 warning full-suite 結果待下一行補記。production canonical release build 已成功；其後變更僅為此 cfg(test) expectation 修正。
- 隨後抓出 fixture 每個 frame 強制 `vertical_scroll_offset(100000)`、未用上次量測 max offset clamp，導致測試刻意把所有內容畫到 clip 外；已依正式 scroll state 的合法範圍先 clamp 再畫。focused edge test 已通過，保留 top code 邊界及 bottom 實際 toolbar paint／glyph-bearing／frame inset assertions；不是修 production 的未知捲動 bug，最後完整 suite 仍另列。
- **最後 source：** `cargo fmt --all -- --check`、`cargo check --locked --all-targets`、`cargo test --locked --all-targets --quiet` 全數通過，無 warning：**435 library passed、2 ignored、23 binary passed**。同一 production rendering 的 canonical release／原生截圖已如上確認；之後僅修 cfg(test) fixture，沒有再重啟／操作使用者 UI。
- 依單一 UI 流程關閉 PID 16020、確認退出／零實例後，不帶 review pin 正常啟動 PID **26008**；只叫出一次。親自讀取 **`unified-icons-no-footer-visible.png`**（實際 940×805 physical pixels，125% 縮放）：五個頂部按鈕同列；Copy/Fork/Retry/Edit 為圖示；原有 CSS／段落正常，左邊框與第一個圖示有安全內距；輸入框下只剩卡片內距，提示列已移除。沒有點 Copy／Fork／Retry／Edit／Send／NewChat，不把 appearance proof 當完整 native action matrix。
- 唯讀偏好欄位確認使用者後來選的 **`%LOCALAPPDATA%\Microsoft\Windows\Fonts\SweiGothicCJKtc-DemiLight.ttf`** 與 TT Norms 均保留並跨重啟實際顯示；這也補上真實使用者安裝字體來源的原生證據。Dark／90%／CursorScreenCenter／Alt+Down 沒有被回復為助理的舊測試偏好。對話未重新送出，沒有 Save API；維持唯一使用者 UI，不再操作當前草稿。

### 字體／視窗設定排版整理

- 使用者回報新偏好區塊雜亂；此輪只整理該區塊，不更改偏好格式、字體套用、熱鍵或位置演算法。
- `ui/preference_layout.rs` 提供一致的 12pt 圓角卡片、16pt 內距、32pt 控制項與 responsive label/control 列。寬版左右對齊，窄版上下堆疊；新增 light/dark × 300/380/520/604pt 幾何測試。
- `fonts::show_font_preferences` 改中文／英文字體各一列，去除雙語重複標籤，字體家族名稱保留完整 hover；即時中英預覽獨立分區，不再顯示工程測試字串。
- `window_prefs::show_preferences` 改視窗卡片；背景不透明度採細滑桿＋可編輯百分比，預設位置與固定座標同列對齊，保留六種模式與「使用目前位置」。
- 最新 `cargo fmt --all`、`cargo check --locked --all-targets`、`cargo test --locked --all-targets --quiet` 通過，無 warning：**408 library passed、2 ignored、23 binary passed**。canonical x64 release build 成功。初次 build 的兩個 float literal warning 已修正並重跑，不把初次有 warning 的結果當最後 source。
- 原生逐次替換 PID 31240 → 32468（排版 prototype）→ 32280（最後 build）。親自讀 prototype 畫面發現 `allocate_ui_with_layout` 不會替短標籤保留 requested width；已用明確 min size 固定欄寬，並加不同長度標籤／控制項右邊緣 regression assertion。卡片內選單加獨立底色與細框，不再像裸文字。
- **最後 build 已親自讀取** `preferences-polished-font-card.png`：604×644 Settings 內中文／英文字體名稱在左，等寬 32pt 選單靠右，分隔線下獨立即時中英預覽，完整卡片邊界。`preferences-redesign-font-prototype.png`／`preferences-redesign-appearance-selected.png` 是較早 prototype（後者含視窗卡片），**不是最後欄寬修正的驗收截圖**。
- 驗收途中實際視窗被切到 chat／hidden；hidden guards 拒絕操作，沒有把 `preferences-polished-window-card.png`（實際 chat）或失敗的 Ctrl+, 當視窗卡片驗收。停止反覆拉回焦點；最後視窗卡片完整原生截圖、其他字體／light theme 原生矩陣尚未補，不以 helper tests 替代。
- 為撤除僅供驗收的 `POPCHAT_PINNED`，正常關閉 PID 32280、等待退出並確認零實例後，以相同最後 executable／profile、不帶 pin／settings 參數正常重啟成 **PID 34176**。確認唯一程序與 hidden rect；不再主動叫出，使用者可 Alt+Down 開啟。此輪未改偏好值、未送聊天／使用 clipboard／按 Save API。

### 六項視窗／字體需求：最新 source 驗證

- `cargo fmt --all`、`cargo check --locked --all-targets`、`cargo test --locked --all-targets --quiet` 通過：**407 library passed、2 ignored、23 binary passed**，無失敗。先前兩個 surface assertion 已修正：解析 egui placeholder 的實際文字色，並同時檢查 press／move frame 的原生拖曳命令。
- `VisibilityController::activate_visible` 由附件輸入 caller 使用：已可見視窗取得焦點不增加 SHOW generation、不重套預設位置；hidden 狀態不自行開啟。新增 regression test 已通過。
- `cargo build --locked --release --target x86_64-pc-windows-msvc` 成功；隨後按使用者授權依序替換 PID 20104 → 26024 → 31240，兩次均先等待舊程序退出並確認零實例，第二次用同一 build 驗證偏好重啟持久化。

#### 同 build 原生操作與親自讀取截圖

- **底部：** `six-features-final-chat.png`／`six-features-final-unpinned.png` 顯示 604×644 原有對話、完整 composer／footer／圓角卡片底部。`six-features-compact-bottom.png` 與 `six-features-compact-multiline.png` 顯示 604×344 空態／三行輸入的完整底欄；`six-features-find-multiline-bottom.png` 顯示 604×644 附件＋三行草稿＋Find 列仍完整。空對話的 Ctrl+F 不開 Find 列，`six-features-compact-find.png` **不算 compact Find 證據**。
- **字體：** 實際選英文 Georgia，親自讀 `six-features-english-georgia.png`，英文變 serif 而中文不變；再選標楷體，`six-features-fonts-georgia-kai.png` 的中文獨立變更，code 仍等寬。只讀這些偏好欄位確認 `georgia.ttf`／`kaiu.ttf` 已即時保存；沒有讀出 key。這不是任意字體匯入／完整 CJK 字形矩陣。
- **透明度：** `six-features-opacity-native.png` 為 55% 背景，底層內容可透出；字／控制項仍清晰不透明。已有上述 renderer 實際文字色 alpha test；不宣稱 DWM glass／blur。
- **拖曳／位置：** 品牌拖曳實際 rect `130,130 → 240,194`；頂部空白帶另移到 `185,167`。固定座標選「使用目前位置」保存 `130,130`，hide/show 後實際回到 `130,130`（`six-features-fixed-show.png`）。再拖離後實際 OLE drop = Copy，rect 仍為 `185,167`、沒有被預設吸回（`six-features-drop-position-after.png`）。只實測 Fixed／Remember，不將六模式 helper tests 當六模式／混合 DPI 原生全驗收。
- **附件 badge：** `six-features-drag-no-position-reset-dropped.png`／`six-features-drop-position-after.png` 親自可見本機 fixture chip 與 composer 上方成功提示，未遮住輸入；臨時 OLE source Form 自動關閉、未改 clipboard、不是第二個 PopChat。
- **快捷鍵：** UI Apply `Ctrl+Alt+P` 後確實 hide/show；短暫 Win32 註冊探針確認舊 Alt+Down 已撤銷，隨即釋放探針。受控註冊 Ctrl+Alt+O 製造真實 OS 衝突，Apply 顯示 error、有效與已存鍵仍 Ctrl+Alt+P（`six-features-conflict-retains-key.png`），舊有效鍵依然可 hide/show；受控衝突註冊在 finally 釋放。單一實例重啟後 Ctrl+Alt+P 仍能 hide/show（`six-features-restarted-custom-show.png`／`six-features-restarted-hotkey.png`）。
- **收尾：** 從 UI 恢復中英 default、100% 背景、Remember、Alt+Down，並再驗證預設鍵 hide/show；視窗移回 `130,130`、取消臨時 pin。原有 Hello 對話仍在，本機測試草稿／附件已清除，未送出聊天、未按 Save API。只有 Fixed 的未使用座標保留 `130,130`；有效位置模式為 Remember。最後唯一程序 PID 31240，已親自讀取 `six-features-final-unpinned.png`。

上述圖片均在 `C:\Users\evis_lai\AppData\Local\Temp\opencode\`。不擴張為真實帳號／完整 IME／效能／多螢幕矩陣驗收。

### 2026-10-30 parent 原生操作

- 測試使用全新隔離 profile `C:\Users\evis_lai\AppData\Local\Temp\opencode\popchat-latest-c2ec38b705cc402490b4f8d37abb6da1`，以及當時的 x64 release build 副本；沒有重用 PID 24032 的 profile。
- 實際服務欄保留未儲存的 endpoint 草稿，再切外觀：點 accentFill／拖 HSB Hue，檢查 `settings.json` 與 `provider_store.json`，已即時保存 bubble style／`#FF0AB9`，沒有提交 API 草稿。Commands system prompt 也不按 Save 即保存 exact 字串。
- 實際拖 compact 右邊框：viewport `674×344 → 794×344`，top 保持 100；`panel.json` 保存 content width 770，保留 expanded height 680，沒有累加 24px shadow padding。
- 本機 SSE fixture 在只有 reasoning 的階段持續送資料；已親自檢查固定 3／12 行視窗、展開按鈕、elapsed label、strip／top fade 與向上捲動後仍停留舊步驟的截圖。串流中輸入下一則草稿並 Enter，HTTP request 仍僅一份；點 Stop 後畫面保留該草稿並恢復 Send。
- 截圖發現等待標題置中，已改左對齊並新增 production renderer regression test；這個最後修正尚未再做原生截圖，不把先前 executable 當最後 source。
- 使用者要求**一次只開一個 UI 實例**後，已以 WM_CLOSE 正常關閉自己建立的 PID 35460 及本機 fixture，確認只留原本 PID 24032；後續不再同時開隔離 UI。先前第二個 profile 造成 Alt+Down 註冊衝突，這些截圖**不是熱鍵成功驗收**。
- 第二次 Send／完成後 settled reasoning、expanded vertical resize、repeat/login hidden、最新視窗的重新開啟持久化仍未完成本輪原生驗收，不能將腳本中途失敗算 PASS。沒有真實 provider／登入／訂閱服務驗證。

已親自讀取的截圖（均在 `C:\Users\evis_lai\AppData\Local\Temp\opencode\`）：`latest-settings.png`、`latest-accent-popup.png`、`latest-accent-dragged.png`、`latest-commands.png`、`latest-reasoning-started.png`、`latest-reasoning-draft.png`、`latest-reasoning-expanded.png`、`latest-reasoning-user-scrolled.png`、`latest-stop-retained-draft.png`。resize 的尺寸及 JSON assertions 由實際 native 操作確認。

### 自動測試／build

- 左對齊修正前：`cargo check --locked --all-targets` 通過；`cargo test --locked --all-targets --quiet`：321 library passed、2 ignored、20 binary passed。兩個 ignored 是 disposable native credential 測試與供其他測試呼叫的 child-process helper，不代表兩個未接線功能。
- 另行執行 `instance::native::tests::native_secondary_process_signals_primary` 通過：無 UI 的真實 child processes 依次送兩次 Show、一次 Settings 並成功退出，不占用熱鍵。
- 最後左對齊修正曾由新增 regression test 抓出 `add_sized` 的 child UI 仍然置中；改成保留完整寬度的 leading layout 後，reasoning production-renderer tests 11 passed。
- 最後 source 已通過 `cargo fmt --all`、`cargo check --locked --all-targets`、`cargo test --locked --all-targets --quiet`：322 library passed、2 ignored、20 binary passed，沒有 warning。這不等於新增原生視窗驗收。
- `cargo build --locked --release --target x86_64-pc-windows-msvc` 成功；最新 executable 為 `target/x86_64-pc-windows-msvc/release/popchat_windows.exe`。沒有啟動此最後 build，也沒有替換／重啟使用者原本的 PID 24032。

### 透明背景陰影修正

- 使用者回報背景有異常陰影後，只讀取目前唯一 PID 24032 的畫面（`shadow-existing-window.png`），沒有開第二個實例、搶熱鍵或重啟使用者視窗。
- 根因：雖然 native viewport／CentralPanel 設為 transparent，App 沒有 override `clear_color`，eframe 0.32 預設仍以 `(12,12,12,180)` 清除整個矩形 framebuffer；這不是卡片圓角陰影。
- 新 `ui/surface.rs` 集中 canvas／card frame，正式 `eframe::App::clear_color` caller 回傳 `[0,0,0,0]`。保留單層較輕的卡片 shadow，blur 16／offset 2／alpha 48，確保包含 stroke 的 shadow reach 在原本 12pt 留白內，不讓陰影在視窗邊界裁成矩形。
- 新 headless regression tests 檢查全透明 clear/canvas 及兩種 theme 的實際 tessellated card/shadow 頂點邊界，2 tests passed。最後 source 的 fmt、all-target check、完整測試與 x64 release build 均通過，沒有 warning：324 library passed、2 ignored、20 binary passed。
- 原本設定頁顯示未儲存變更；先詢問後，使用者明確選擇「捨棄草稿並重啟」。已 WM_CLOSE 關閉 PID 24032，等待退出並確認零實例後，才使用原 profile 開啟該次最後 x64 release：**當時唯一 PID 23784**。沒有自動 Save API 草稿、沒有並行 UI、沒有丟棄對話資料；PID 23784 從此視為使用者的活躍視窗，不得當成可任意關閉的無人 fixture。
- 第一次啟動後 capture 尚未找到 HWND，後續讀到 hidden；只對確認唯一的實例按一次 Alt+Down 叫回。已親自讀取 `shadow-fixed-settings.png`（實際為 chat 畫面）及 `shadow-same-position-after.png`；沒有把 capture 失敗或 hidden 狀態算驗收成功。
- 較早的 desktop baseline 因底層畫面變更而有 18 級像素差，未算 PASS；改成同一視窗位置直接 hide → 擷取背景 → show 比較：左上／右上／左下角、左邊及頂邊中段與背景 RGB 完全一致，右下角差 4 級（仍有輕微 shadow／底層變動），不宣稱四角零差或已驗證所有 DWM 狀態。這證實不再整塊套用 alpha-180 深色 framebuffer 底。
- Native evidence：`shadow-same-position-background.png`、`shadow-same-position-after.png`、`shadow-fixed-settings.png`；都在前述 opencode temp 目錄。保留正常卡片柔和陰影；不是宣稱 glass／DWM／混合 DPI 完整 parity。

### 按鈕比例修正

- 使用者截圖指出 chat 的新增／複製／分支／Retry／Edit 與 composer 附件按鈕被壓扁。這些入口原本呼叫 `small_button`／`Button::small`，egui 會刻意移除垂直 padding；不是整個介面需要放大。
- 新 `ui/controls.rs::button/toggle` 明確固定 compact 高度 26pt、左右 9pt／上下 4pt padding、6pt corner radius；正式 chat 導覽與 message toolbar caller 已替換，fork disabled／copy feedback／streaming toolbar reserve 等行為保留，不改全域字級或 settings 按鈕比例。
- Composer inline 附件入口改用 `Icon::Add` vector 加號，28×28 hit target 與 Expand／Send／Stop 一致；不再將文字全形加號塞進扁長 small button。附件 chips／expanded attachment action 也改用有 padding 的 compact control。
- 新 headless tests 覆蓋 dark/light 文字實際 galley 與按鈕高度／留白、點擊垂直 padding、樣式不外漏，以及 header／composer icon square hit targets。`cargo fmt --all`、`cargo check --locked --all-targets`、UI tests（145 passed）及完整測試（327 library passed、2 ignored、20 binary passed）通過；release build 仍在執行，不把 source caller／headless tests 當原生驗收。
- 使用者另行明確選擇「捨棄草稿並重啟」，允許本次捨棄未送出輸入及未儲存設定；只在 build 成功後 WM_CLOSE 目前唯一 PID 23784，確認零實例後才開新 build。未啟動第二個 UI，也未自行提交 API 草稿。
- 為避免覆寫執行中的 release exe，本輪 build 使用 `target/button-review`，已成功。使用者同意後，舊 PID 23784 已正常退出並確認零實例，才啟動新 **PID 4188**；從此視為使用者的活躍視窗，不得當成可任意關閉的 fixture。
- 已親自讀取 `buttons-new-window.png`（Settings）、`buttons-actual-current.png`（compact chat）。後者可見正常 26pt 新聊天／歷史／尋找按鈕及方形加號 hit target；沒有送出測試訊息、改服務或讀取 API key。兩次因視窗 hidden 而拒絕的操作未算 PASS。使用者後續截圖也顯示新比例，另回報下節的可視性／submenu 捲動問題。

### 歷史可視性／模型子選單滾輪（較早 snapshot）

- 使用者截圖顯示 history selected row 使用飽和 accent，白字／secondary 對比不足、title 被 `add_sized(Button)` 置中、delete 又是扁的 small button；provider submenu 的 model list 則沒有自己的 ScrollArea，超出 compact native viewport 後，視窗外的滾輪事件無法送回 app。
- Compact selected navigation 已改成淡 accent tint 與明確 primary foreground，避免 inherited theme-white text 配 bright green／yellow。新增實際 paint 色彩與 light/dark／五種 custom accent 的對比測試。
- `main::show_history → ui/history.rs::row` 已接實際 caller：title／preview 左對齊、selected/current 使用 8–15% tint、普通文字必要時提升對比至 4.5:1，delete 為獨立 26×26 vector target，整列及 preview/padding 可開啟而 delete 不會誤開聊天。History renderer 5 tests passed；固定 row 高度 42/62 與原版 30/46 不同，不假稱像素完全相同。
- `main::header_row → switcher::model_list` 已接實際 caller：root popup 明確限制 width／provider scroll 高度，child model scroll 依 native viewport 高度保留 140pt 空間，上限 360pt；文字過長 truncate／hover 完整名稱，provider-specific scroll ID，model／effort 的原始字串照原樣回到 `ProviderStore::select`。5 個 helper tests 含 272/344 高度的真正 nested popup wheel event／末列可達／exact model及effort選擇，完整主程式整合與 native 結果另記。
- 選單會限定在 native viewport 裡，不改動原本 hotkey／blur auto-dismiss 契約，避免因 popup 殘留而失焦永不隱藏。Native 驗證仍只允許一個 UI，不會另開隔離實例。
- 使用者在本輪詢問中明確回覆「都可以直接重啟」；更新仍必須先 WM_CLOSE 舊版、等待退出且確認零實例後才開新 build，不自行 Save API 草稿。當時唯一實例是 PID 4188，後續替換另記新 PID。
- 已以相同 profile 單一替換 `4188 → 30624`。該 snapshot 完整驗證為 156 UI passed、338 library passed／2 ignored、20 binary passed，fmt／all-target check／release build 成功。已親自檢查 `visibility-history-fixed.png`：左對齊 title／preview、淡色選取列、較大的 vector delete target 可見；底色不再整列飽和綠。
- Native 滾輪嘗試中 UI 曾 hidden，已由 guard 拒絕後續點擊／拒絕宣稱成功；`menu-wheel-root.png` 僅證明 root menu 位置，不代表 child wheel 已原生通過。headless 的真實 nested-popup wheel tests 與尚待補的 native 操作仍分開列示。
- Native history 截圖又發現目前聊天的 `✓` 字型 fallback 出現空方框；已把 history／model-list check 改為 vector，並修正 regression test 區分 delete X 與 current check 的各兩條線，非只放寬總線段數。

### 使用者調整：移除尋找按鈕／快捷鍵／檔案貼上與拖入動畫

- 明確依使用者要求移除常駐「尋找」按鈕：正式 caller 改用 `controls::chat_navigation`，只畫新聊天／歷史；`Ctrl+F`、`Ctrl+G`／`Ctrl+Shift+G` 仍有原本 Find state/action caller。新增實際 renderer 測試確認沒有「尋找」按鈕，不是刪掉 Find 功能。
- 補上 `Ctrl+O` 原生多檔 picker；`Ctrl+V`／`Shift+Insert` 貼上、Ctrl+N/Y/E 等快捷鍵在 Settings 快捷鍵頁列出，composer footer 有貼上／拖入提示。當時全域固定 Alt+Down；最新需求已改可設定、預設 Alt+Down，Alt+Space 仍不註冊。
- 找到原本檔案／image-only paste 漏接的根因：egui-winit 0.32 把 Ctrl+V 吃掉，只有 clipboard 有文字時才送 `Event::Paste`。新 `ui/paste.rs` 透過 winit Windows message observer，只觀察目前 root HWND 的 Ctrl+V／Shift+Insert／WM_PASTE，request repaint 後讓原訊息照常 dispatch，不安裝全域鍵盤 hook、不讀取 clipboard、不吞文字貼上。
- `chat/clipboard.rs::file_paths` 在實際 paste 時快照 CF_HDROP（不是把貼上文字猜成路徑），檔案資料沿既有 worker loader → active composer → send wire。只有 format available 才開 clipboard；Unicode／長路徑／UNC／不合法 UTF-16 code unit 保留；格式／筆數／記憶體長度有界限，失敗有可見錯誤。普通文字沒有 image formats 時不啟動讀圖 task／動畫。
- 實際 drop caller 接收 path 與 in-memory `DroppedFile.bytes`，都走相同 `attach::load_bytes` canonical parser；文字／PDF／RTF／圖片的 byte/path parity 與混合成功／失敗 drop batches 已有測試。仍不宣稱 Windows Outlook virtual-file promises／FileDescriptor 型別已支援。
- `ui/file_drop.rs::FileDropFeedback` 已接 `show_card` 後的 paint-only caller：hover 淡入／scale／glow／bounce、loading、實際成功 append 後的 2 秒 badge；0 個成功不會顯示成功，沒有新增可攔截 click／wheel 的 overlay widget。僅 active/fade/pulse 時有界 30Hz repaint，`animation_time==0` 為靜態，非已自動讀 OS Reduce Motion。
- 檔案 hover／drop 期間暫停 blur dismiss，drop 後用既有 serialized show/native activation 路徑把 focus 回 draft；沒有合成 Alt-up。貼上／drop 以 frame ID fencing 避免 egui 重排 pass 重複載入；已處理 CF_HDROP 的同幀 text paste 不會另插入路徑文字。
- 首次整合通過 365 library／2 ignored、22 binary；vector marker 檢查曾因舊 test 預期 2 條線、實際包含 delete X 與 current check 共 4 條而失敗，後來精確按 delete rect 區分後重跑。六項新需求之前最後完整 snapshot 為 **365 library passed／2 ignored、22 binary passed**，x64 `target/button-review` release build 成功；這不是六項新 source 的結果。

### Parent 回報的原生 file／menu 證據（六項改動前）

- 使用者授權的重啟仍遵守先關閉舊 UI、等待退出／零實例才開新 UI，最後只留使用者活躍 **PID 20104**；未並行啟動第二個 PopChat。以下為 parent 已取得的證據，此文件審查未重做 native 操作／讀取 clipboard。
- **檔案貼上：** 普通 Ctrl+V、clipboard **只有 CF_HDROP、沒有 Text**，實際出現附件 chip 與成功 badge：`file-native-paste-result.png`。根目錄測試檔及原始 clipboard backup 該次已還原；不能泛稱所有測試都還原 clipboard。
- **普通文字：** `file-native-text-result.png` 可見一般文字 prefix。中文 fixture 因 PowerShell UTF-8 source 先被破壞，不能當 app 中文貼上壞掉或正常的證據。第二次測試偵測 clipboard sequence 已變更，保留較新資料、沒有以舊 backup 覆寫。
- **真實 OLE drop：** 初次 hidden source 的 `DoDragDrop` 結果為 **None，不算 PASS**。後改用可見的臨時 source Form（不是第二個 PopChat），結果 **Copy**，source Form 自動關閉；`file-native-ole-real-hover-1.png`／`file-native-ole-real-hover-2.png` 顯示 hover 動畫，`file-native-ole-real-dropped.png` 顯示實際附件 chip。僅證明真實路徑 OLE，不代表 FileDescriptor／virtual-file promise。
- 原 temporary native harness 後改 marker-aware，只影響 temp 測試碼、不是產品修正。舊 snapshot 的成功 badge 曾短暫覆蓋 composer；最新 `main::show_composer_capsule → FileDropFeedback::set_composer_rect` 已改錨定 composer 上方，headless regression test 與上方最新同 build 原生截圖均確認。
- **模型滾輪：** pin 狀態下實際 **604×344 compact viewport**，滑鼠在 model rows 而非 scrollbar 上滾動；`model-wheel-compact-top.png` 可見 `codex-auto-review/gpt5.5`，`model-wheel-compact-bottom.png` 可見 `gpt6-sol/gpt6.1-sol/gptreserve`，已達最後列，沒有點選任何模型。這補足前述只有 root menu／hidden guard 的未完成嘗試，不抹除那些失敗紀錄。
- **Find：** `file-input-find-shortcut.png` 證明 Ctrl+F 開 Find bar；`file-input-no-find-button.png` 證明 chat 無常駐 Find 按鈕，非刪除功能。上述截圖位於既有 opencode temp 目錄，不是最新六項 build 的證據。

### 最新六項需求：接線摘要（驗證結果見上）

- 最新 caller 已由靜態來源核對，見上表及 `FUNCTION_PARITY.md`；涉及 `main::new/update/settings_actions/show_card/header_row`、`ui/{chat_layout,fonts,hotkey,window_prefs,surface,settings}.rs`。`font_preferences/window_preferences` 由明確 allowlist 獨立 autosave；API endpoint/key/model 草稿與未知 JSON 欄位保留，hotkey draft 不搭便車提交。
- 預設位置六模式為：記住上次、游標螢幕中央、游標附近、右上、右下、固定座標（可取目前位置）。只在 startup／新 SHOW 套用；monitor change 只 clamp 現有拖曳位置，不每幀吸回預設。
- Footer measured/bounded layout、字型組合／失敗保留、hotkey transaction、背景 alpha／drag region、placement 各有測試；完整結果為 407 library passed、2 ignored、23 binary passed，上方列出同 build 有限原生證據與仍未驗收矩陣。PID 20104 已退出，最新唯一 UI 為 PID 31240。
- 文件交付範圍只有兩份 Markdown；清冊仍保留 **32 files／513 definitions（450 + 63）** 與全部原始行號／definition IDs，另以 read-only 對照核對。剩餘缺口仍包含真實帳號／效能／IME／math 相容性／virtual promises 與完整 native matrix，不提供完成百分比。
