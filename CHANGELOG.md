# v0.2.0 — LOCAL BUILD

Windows Rust 移植／維護：**Evis Lai**。
原專案：[lec77/PopChat](https://github.com/lec77/PopChat)，作者 **Le Chen**，MIT。

## 本版重點

- 新增正式專案 README、原作者標記與完整的發行授權文件。
- 更新來源改為 `lsyuan36/popchat_windows` 的 GitHub Releases。
- 支援程式內下載更新，驗證 GitHub 資產 SHA-256，經使用者確認後以獨立
  Rust 更新助手等待目前程式退出、安裝並重新啟動；保留偏好、服務與歷史。
- 發行檔為 Windows x64 ZIP，包含聊天程式、更新助手及第三方授權聲明。
- 內建獅尾圓體中文預設；英文預設優先使用已安裝的 TT Norms Pro，再使用 Arial。
  **不散佈 TT Norms Pro 或 Arial。**
- 新增 12–24 pt 聊天文字大小、快捷鍵錄製、分組鍵帽式快捷鍵清單。
- 圖示 hover 顯示功能與快捷鍵，停用狀態顯示原因。
- 加入頁面／設定子分頁轉場與視窗高度動畫。
- 改善訊息工具列、緊湊間距、表格欄寬／列高與整表 Markdown 複製。

## 下載

下載 **`PopChat-Windows-x64.zip`**，完整解壓縮後執行 `popchat_windows.exe`。
`popchat_updater.exe` 必須保留在同一資料夾；不需系統管理員權限。
`SHA256SUMS.txt` 與 `release-manifest.json` 提供檔案雜湊和來源 commit。

## 注意事項

- 這是獨立、未簽章的 LOCAL BUILD，並非原 macOS 專案的官方發行。
- SHA-256 驗證是完整性檢查，不是發行者數位簽章。
- 重新啟動前請完成尚未送出的訊息與未儲存的 API 草稿；不會擅自提交它們。
- Windows 原生輸入法、完整混合 DPI／無障礙，以及所有真實供應商帳號的驗收仍未完成。
- 新增更新流程有離線與隔離檔案交易測試；本版是第一個公開發行版本，
  尚無更高版本可供真實帳號的「下一版更新」驗收。
