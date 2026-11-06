# Skills 技能設定（0.2.9）

此功能自 **0.2.6** 正式提供；**0.2.4** 及更早版本需升級，既有安裝不會因原始碼修改自行變更。

## 使用方式

1. 設定 → **Skills** → **管理技能／掃描來源**。
2. 核對來源資料夾，按 **掃描／重新整理技能**。只讀取本機資料，不呼叫模型或執行腳本。
3. 查看名稱、說明與完整檔案位置，個別勾選信任的技能。
4. 下一次送出聊天時生效；不需要修改自訂 system prompt。

**勾選啟用代表同意將該技能指引及目錄內文字參考檔，按任務需要傳送給目前模型服務。**
請勿在技能或參考檔放入憑證及不希望傳送的資料。來源勾選僅決定掃描範圍，
新技能仍預設停用；來源停用時，其技能不會提供給模型。

啟停或修改來源會停止目前工具回合，下一次送出才使用新設定。
技能本身的內容／metadata 變更後會自動失效，須重新掃描並啟用；不默默沿用舊授權。
參考檔在啟用的技能目錄內按需讀取最新內容，不另逐檔詢問，請只啟用可信任的整個技能目錄。

## 預設與自訂路徑

`~` 指目前使用者的主目錄；Windows 通常是 `%USERPROFILE%`。

| 來源 | 個人預設路徑 |
|---|---|
| OpenCode | `~/.config/opencode/skills`；有效的絕對 `XDG_CONFIG_HOME` 會取代 `~/.config` |
| Claude | `~/.claude/skills` |
| Codex／共用 Agents | `~/.agents/skills` |
| Codex 舊版相容 | `~/.codex/skills`，不是目前官方主要路徑 |

「加入專案／自訂來源」接受絕對資料夾路徑：

- 自訂：掃描該資料夾。
- 專案：只加入其 `.opencode/skills`、`.claude/skills`、`.agents/skills`。
- 不自動遍歷其他專案、向上搜尋 repo、讀取其他工具設定、匯入外掛技能或下載內容。
- 缺少的預設目錄會略過。移除來源只修改 PopChat 設定，不刪除檔案；保守清除所有技能授權，須重新勾選。

來源與授權存在 PopChat 資料目錄的 `skills.json`，不混入一般偏好、服務、憑證或聊天歷史。
檔案無法讀取或格式錯誤時會停用 Skills 並禁止覆寫原設定。

## 格式與相容範圍

```text
skills/
  example/
    SKILL.md
    references/
      guide.md
```

```markdown
---
name: Example Skill
description: 適用於什麼任務，可使用 YAML 多行文字。
---
技能指引。必要時閱讀 references/guide.md。
```

- 使用 YAML parser，支援引用字串、多行文字、CRLF 與 UTF-8 BOM；`name`／`description` 必須為非空字串。
- 支援巢狀 `SKILL.md`，最深 6 層；OpenCode／自訂來源另支援根目錄 `*.md`。
- 相同名稱不互相覆寫，以完整檔案路徑產生獨立 ID；介面顯示檔案位置供區分。
- `disable-model-invocation: true` 不提供給模型。此版沒有 `/技能名稱` 手動呼叫功能。
- 不執行 hooks、動態 shell、腳本、安裝或下載；`allowed-tools` 不授權任何工具；`context: fork` 不建立子代理。這不是完整 CLI 執行環境。
- 技能要求未提供的 shell、寫檔等能力時，模型應說明限制，不能假裝已完成。

## 模型與上下文

- 適用於支援 function calling 的 **OpenAI 相容服務**與 **ChatGPT Responses**。
- Codex app-server 不注入 PopChat 的 Skills 工具，也不替它修改原生設定。
- 名稱／說明及 ID 透過 `load_skill` 的工具說明提供；全文不會預載至 system prompt。
- 模型呼叫 `load_skill` 才讀取全文；載入後可用 `read_skill_file` 讀取該技能內的文字參考檔。
- 根目錄 `*.md` 沒有獨立支援資料夾，因此不能讀取同層參考檔。
- 目錄、載入全文與工具結果都會占模型上下文；工具往返不額外寫入永久聊天歷史。技能不會覆寫自訂 prompt，也不高於使用者／系統要求。
- 一般聊天沒有啟用技能時不掃描技能來源；在管理頁明確按下掃描才探索新技能。

## 安全與上限

- 每檔最多 32 KiB、frontmatter 最多 8 KiB，僅 UTF-8 文字。
- 單次掃描最多 4096 個目錄項目、256 個技能；最多 32 個來源。
- 單輪最多 48 個技能，兩個技能工具的序列化 schema 合計不超過 24 KiB；超出會按名稱／ID 排序截選，介面顯示警告。應停用不需要的技能。
- 單輪最多 16 次技能讀取；仍受 provider 的整體工具回合上限約束。
- 參考檔只允許技能內相對路徑，使用 `/`；支援 `.md`、`.txt`、`.json`、`.yaml`、`.yml`、`.toml`、`.csv`。拒絕絕對路徑、`..`、隱藏檔、NTFS stream、符號連結與 junction。
- Windows 讀取時鎖定目錄的刪除／重新命名分享權限，阻擋檢查後替換連結；檔案讀取拒絕同時寫入／替換。
- 啟用技能只是允許讀取與提供指引，不是沙箱、任意檔案存取或工具授權；MCP 工具仍須每次確認。
- 已儲存的 MCP 憑證也會從技能目錄與讀取結果中遮罩，但這不是通用機密偵測器。

## 格式參考

- [OpenCode V2 Skills](https://opencode.ai/v2/docs/skills)
- [Codex Skills](https://developers.openai.com/codex/skills)
- [Claude Code Skills](https://code.claude.com/docs/en/skills)

測試僅使用臨時假技能、記憶體憑證與本機 mock 模型；不讀取使用者真實技能、
不啟動聊天 UI、不送真實聊天、不修改剪貼簿或其他工具設定。

```powershell
cargo check --locked --all-targets
cargo test --locked --all-targets --quiet -- --test-threads=4
```

本機完整回歸採 4 條測試執行緒，降低既有 Windows 子程序 fixture 在高併發下的
啟動逾時；不改動產品的逾時或程序管理邏輯。另以離線 egui 預覽檢查深／淺色與窄視窗。

本輪完整回歸：**620 library、29 app、1 send-pipeline integration、1 license inventory、
15 importer，合計 666 項通過、3 項忽略，無 warning**。
Skills 的 API／Responses 工具往返、明確勾選保存、延遲掃描失效、內容變更、
目錄範圍／junction、憑證遮罩及大小限制均以隔離 fixture 驗證。
新增 YAML 依賴的完整授權整合於既有第三方通知；打包前的授權迴圈驗證通過，
拒絕損毀雜湊，仍維持 **877 個 ZIP 項目**；發行成品以對應 tag CI 的 manifest 與 SHA-256 為準。

`v0.2.5` 候選沒有正式 Release；CI 揭露既有測試對選用系統楷體的依賴，
已改用內建且授權完整的字體 fixture，保留原有斷言，不安裝字體或略過測試。
