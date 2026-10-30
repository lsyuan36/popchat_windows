//! Read a snapshot of Explorer's copied files without consuming the clipboard.
//!
//! Only CF_HDROP is inspected; clipboard text is never interpreted as paths.
//! Paths (including directories and duplicates) retain their original order and
//! spelling. File loading, validation, and any deduplication belong to the caller.

use std::path::PathBuf;

/// Snapshot copied file paths, or `None` when CF_HDROP is unavailable.
///
/// This performs no file I/O or content decoding. On Windows the clipboard is
/// opened once, without retries; a busy clipboard is an error, not an absent
/// format. The clipboard and its OS-owned data are never changed or freed.
pub fn file_paths() -> anyhow::Result<Option<Vec<PathBuf>>> {
    #[cfg(windows)]
    {
        windows::file_paths()
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

/// Probe only on an actual paste request. These are the two formats supported
/// by arboard's Windows image reader; ordinary text never starts a decode task.
pub fn image_available() -> bool {
    #[cfg(windows)]
    {
        windows::image_available()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

// Keep the bounded query algorithm shared by the real API adapter and tests.
#[cfg(any(windows, test))]
mod drop_query {
    use anyhow::{ensure, Context, Result};

    pub(super) const MAX_DROP_BYTES: usize = 25 * 1024 * 1024;
    const MAX_PATHS: u32 = 4096;

    /// Matches DragQueryFileW: UINT_MAX queries count; a null buffer queries
    /// length excluding NUL; a supplied buffer includes space for the NUL.
    pub(super) trait DropQuery {
        fn query(&self, index: u32, buffer: Option<&mut [u16]>) -> u32;
    }

    pub(super) fn snapshot(query: &impl DropQuery) -> Result<Vec<Vec<u16>>> {
        let count = query.query(u32::MAX, None);
        ensure!(count > 0, "CF_HDROP contains no readable file paths");
        ensure!(
            count <= MAX_PATHS,
            "CF_HDROP contains too many paths (limit {MAX_PATHS})"
        );
        let mut paths = Vec::with_capacity(count as usize);
        let mut total_bytes = 0usize;
        for index in 0..count {
            let length = query.query(index, None);
            ensure!(length > 0, "CF_HDROP path {index} is empty or unreadable");
            let capacity = length
                .checked_add(1)
                .context("CF_HDROP path length overflow")?;
            let capacity =
                usize::try_from(capacity).context("CF_HDROP path length exceeds usize")?;
            let bytes = capacity
                .checked_mul(2)
                .context("CF_HDROP path size overflow")?;
            total_bytes = total_bytes
                .checked_add(bytes)
                .context("CF_HDROP snapshot size overflow")?;
            ensure!(
                total_bytes <= MAX_DROP_BYTES,
                "CF_HDROP paths exceed the 25 MiB snapshot limit"
            );

            // A nonzero sentinel detects an API response missing its terminator.
            let mut path = vec![0xffff; capacity];
            let copied = query.query(index, Some(&mut path));
            ensure!(
                copied == length,
                "CF_HDROP path {index} changed length or could not be read"
            );
            ensure!(
                path[capacity - 1] == 0,
                "CF_HDROP path {index} is not NUL-terminated"
            );
            path.pop();
            ensure!(
                !path.contains(&0),
                "CF_HDROP path {index} contains an embedded NUL"
            );
            paths.push(path);
        }
        Ok(paths)
    }
}

#[cfg(windows)]
mod windows {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, ptr};

    use anyhow::{ensure, Context, Result};

    use super::drop_query::{self, DropQuery, MAX_DROP_BYTES};

    type Handle = *mut std::ffi::c_void;
    const CF_HDROP: u32 = 15;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn DragQueryFileW(drop: Handle, index: u32, buffer: *mut u16, capacity: u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalSize(memory: Handle) -> usize;
    }

    struct Clipboard;

    impl Clipboard {
        fn open() -> Result<Self> {
            // SAFETY: a null owner is valid for read-only clipboard access.
            if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Could not open the clipboard (it may be busy); try pasting again");
            }
            Ok(Self)
        }
    }

    impl Drop for Clipboard {
        fn drop(&mut self) {
            // SAFETY: this guard exists only after a successful OpenClipboard.
            unsafe { CloseClipboard() };
        }
    }

    // The borrowed clipboard guard keeps the OS-owned HDROP valid throughout
    // querying. Do not GlobalFree/DragFinish this handle: we do not own it.
    struct NativeDrop<'a> {
        handle: Handle,
        _clipboard: &'a Clipboard,
    }

    impl DropQuery for NativeDrop<'_> {
        fn query(&self, index: u32, buffer: Option<&mut [u16]>) -> u32 {
            let (pointer, capacity) = match buffer {
                Some(buffer) => {
                    // The shared snapshot routine bounds lengths before this.
                    let Ok(capacity) = u32::try_from(buffer.len()) else {
                        return 0;
                    };
                    (buffer.as_mut_ptr(), capacity)
                }
                None => (ptr::null_mut(), 0),
            };
            // SAFETY: handle comes from CF_HDROP while the clipboard is open;
            // the writable buffer, if present, has exactly capacity u16 units.
            unsafe { DragQueryFileW(self.handle, index, pointer, capacity) }
        }
    }

    pub(super) fn file_paths() -> Result<Option<Vec<PathBuf>>> {
        // No clipboard lock for a normal text/image paste. Recheck after open
        // as another process can replace the formats between these operations.
        if unsafe { IsClipboardFormatAvailable(CF_HDROP) } == 0 {
            return Ok(None);
        }
        let clipboard = Clipboard::open()?;
        // SAFETY: checking a predefined clipboard format has no side effects.
        if unsafe { IsClipboardFormatAvailable(CF_HDROP) } == 0 {
            return Ok(None);
        }
        // SAFETY: the clipboard is open and the format was advertised.
        let handle = unsafe { GetClipboardData(CF_HDROP) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("Could not read the clipboard's CF_HDROP file list");
        }
        // CF_HDROP is a global-memory DROPFILES block. Bound its source size as
        // well as the converted UTF-16 snapshot; DragQueryFileW handles both
        // Unicode DROPFILES and legacy ANSI using Windows' native conversion.
        // SAFETY: handle is the clipboard's live CF_HDROP global-memory handle.
        let size = unsafe { GlobalSize(handle) };
        if size == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Could not determine the clipboard's CF_HDROP data size");
        }
        ensure!(
            size >= 20,
            "CF_HDROP data is shorter than its DROPFILES header"
        );
        ensure!(
            size <= MAX_DROP_BYTES,
            "CF_HDROP data exceeds the 25 MiB snapshot limit"
        );
        let query = NativeDrop {
            handle,
            _clipboard: &clipboard,
        };
        let paths = drop_query::snapshot(&query)?
            .into_iter()
            // Preserve even unpaired UTF-16 surrogates (no lossy String step).
            .map(|path| PathBuf::from(OsString::from_wide(&path)))
            .collect();
        Ok(Some(paths))
        // The clipboard guard is released on success and every error path.
    }

    pub(super) fn image_available() -> bool {
        const CF_DIBV5: u32 = 17;
        static PNG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        // Registering a format name does not read or change clipboard contents.
        let png =
            *PNG.get_or_init(|| unsafe { RegisterClipboardFormatW([80u16, 78, 71, 0].as_ptr()) });
        unsafe {
            IsClipboardFormatAvailable(CF_DIBV5) != 0
                || (png != 0 && IsClipboardFormatAvailable(png) != 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::drop_query::{snapshot, DropQuery, MAX_DROP_BYTES};

    struct Fixture {
        paths: Vec<Vec<u16>>,
        count: Option<u32>,
        length: Option<u32>,
        copied: Option<u32>,
        terminate: bool,
    }

    impl Fixture {
        fn new(paths: Vec<Vec<u16>>) -> Self {
            Self {
                paths,
                count: None,
                length: None,
                copied: None,
                terminate: true,
            }
        }
    }

    impl DropQuery for Fixture {
        fn query(&self, index: u32, buffer: Option<&mut [u16]>) -> u32 {
            if index == u32::MAX {
                return self.count.unwrap_or(self.paths.len() as u32);
            }
            let path = &self.paths[index as usize];
            let Some(buffer) = buffer else {
                return self.length.unwrap_or(path.len() as u32);
            };
            let copied = path.len().min(buffer.len());
            buffer[..copied].copy_from_slice(&path[..copied]);
            if self.terminate && copied < buffer.len() {
                buffer[copied] = 0;
            }
            self.copied.unwrap_or(path.len() as u32)
        }
    }

    fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().collect()
    }

    #[test]
    fn preserves_unicode_order_duplicates_and_path_spelling() {
        let paths = vec![
            wide(r"C:\資料\報告🦀.pdf"),
            wide(r"\\server\share\folder.rtfd"),
            wide(r"\\?\C:\Windows\System32\..\example.txt"),
            wide(r"C:\folder\"),
            wide(r"C:\資料\報告🦀.pdf"),
        ];
        assert_eq!(snapshot(&Fixture::new(paths.clone())).unwrap(), paths);
    }

    #[test]
    fn preserves_unpaired_surrogates_and_paths_longer_than_max_path() {
        let mut path = wide(r"\\?\C:\");
        path.extend(std::iter::repeat_n(b'a' as u16, 1000));
        path.extend([0xd800, b'.' as u16, 0xdc00]);
        assert_eq!(
            snapshot(&Fixture::new(vec![path.clone()])).unwrap(),
            vec![path]
        );
    }

    #[test]
    fn rejects_empty_and_excessive_counts_before_reading_paths() {
        assert!(snapshot(&Fixture::new(vec![])).is_err());
        for count in [4097, u32::MAX] {
            let mut fixture = Fixture::new(vec![]);
            fixture.count = Some(count);
            assert!(snapshot(&fixture).is_err());
        }
    }

    #[test]
    fn accepts_count_limit() {
        let paths = vec![wide(r"C:\x"); 4096];
        assert_eq!(snapshot(&Fixture::new(paths.clone())).unwrap(), paths);
    }

    #[test]
    fn rejects_empty_overflowing_and_over_budget_lengths() {
        for length in [0, u32::MAX, (MAX_DROP_BYTES / 2) as u32] {
            let mut fixture = Fixture::new(vec![wide(r"C:\x")]);
            fixture.length = Some(length);
            assert!(snapshot(&fixture).is_err());
        }
    }

    #[test]
    fn enforces_aggregate_budget_not_just_each_path() {
        let path = vec![b'a' as u16; MAX_DROP_BYTES / 4];
        let error = snapshot(&Fixture::new(vec![path.clone(), path])).unwrap_err();
        assert!(error.to_string().contains("25 MiB"));
    }

    #[test]
    fn rejects_failed_or_inconsistent_copies() {
        for copied in [0, 1, 100] {
            let mut fixture = Fixture::new(vec![wide(r"C:\x")]);
            fixture.copied = Some(copied);
            assert!(snapshot(&fixture).is_err());
        }
    }

    #[test]
    fn rejects_missing_terminators_and_embedded_nuls() {
        let mut fixture = Fixture::new(vec![wide(r"C:\x")]);
        fixture.terminate = false;
        assert!(snapshot(&fixture).is_err());
        assert!(snapshot(&Fixture::new(vec![vec![b'C' as u16, 0, b'x' as u16]])).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_conversion_is_lossless() {
        use std::{
            ffi::OsString,
            os::windows::ffi::{OsStrExt, OsStringExt},
            path::PathBuf,
        };
        let mut units = wide(r"\\?\UNC\server\share\資料\");
        units.extend([0xd800, 0xdc00, 0xdfff]);
        let snapshot = snapshot(&Fixture::new(vec![units.clone()])).unwrap();
        let path = PathBuf::from(OsString::from_wide(&snapshot[0]));
        assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), units);
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_has_no_file_clipboard() {
        assert!(super::file_paths().unwrap().is_none());
    }
}
