//! 安全診斷輸出：絕對不 panic 的 `println!` 替代品。
//!
//! 為什麼要這個：`println!` 在標準輸出是壞掉的 pipe 時會直接 panic
//! （os error 232，Windows 上把 stdout 重導到已關閉的 pipe/檔時會發生）。
//! 本專案診斷線（visibility-driver）一旦在 `println!` panic，執行緒就死，
//! 全域熱鍵事件再也沒人處理 → 使用者看到的是「熱鍵沒有反應」，其實是
//! driver 已掛掉。實測踩過（run22.log）。
//!
//! `logf!` 一律 best-effort 寫檔（`%APPDATA%/PopChat/config/logs/popchat.log`）。
//! 不寫 stdout：仍開啟卻沒有人讀的 pipe 會阻塞 write_all，卡住熱鍵與 UI。

use std::io::Write;
use std::sync::OnceLock;

/// 取得 log 檔位置（有 ProjectDirs 時）。
fn log_file() -> Option<&'static std::path::PathBuf> {
    static FILE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    FILE.get_or_init(|| {
        #[cfg(test)]
        {
            static TEST_DIR: OnceLock<Option<tempfile::TempDir>> = OnceLock::new();
            TEST_DIR
                .get_or_init(|| tempfile::tempdir().ok())
                .as_ref()
                .map(|dir| dir.path().join("popchat.log"))
        }
        #[cfg(not(test))]
        crate::paths::config_dir().map(|dir| dir.join("logs").join("popchat.log"))
    })
    .as_ref()
}

/// 寫一行診斷。任何 I/O 失敗都靜默忽略，絕不 panic。
pub fn log_line(args: std::fmt::Arguments<'_>) {
    log_line_file_only(args);
}

/// 只寫檔（panic hook 用：panic 期間不能再碰 stdout）。
fn log_line_file_only(args: std::fmt::Arguments<'_>) {
    let line = format!("{}\n", args);
    if let Some(path) = log_file() {
        write_file_at(path, &line);
    }
}

fn write_file_at(path: &std::path::Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

/// 安裝 panic 鉤子：任何執行緒 panic 都記錄到 log 檔（含線程名），
/// 讓「driver 死了但視窗還在」這類狀態可取證。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        log_line_file_only(std::format_args!("[panic][{name}] {info}"));
    }));
}

/// `println!` 的安全替代品：永不 panic、診斷寫檔。
#[macro_export]
macro_rules! logf {
    ($($arg:tt)*) => { $crate::log::log_line(std::format_args!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_log_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        // Opening a directory as an append-only file must fail harmlessly.
        write_file_at(dir.path(), "[test] unavailable destination\n");
        assert!(dir.path().is_dir());
    }

    #[test]
    fn writes_to_file_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("popchat.log");
        write_file_at(&path, "isolated log test\n");
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "isolated log test\n"
        );
    }
}
