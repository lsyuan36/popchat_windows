//! Per-profile Windows equivalent of `applicationShouldHandleReopen`.
//!
//! A second process signals SHOW (never toggle), or Settings for `--settings`,
//! then exits. Keep the primary guard alive for the entire app lifetime and poll
//! `take_request` from the native driver even while the egui window is hidden.
//! No window enumeration, process scanning, registry writes, or worker thread.

/// The second process has already delivered its request when `Reopened` returns.
#[derive(Debug)]
pub enum InstanceLaunch {
    Primary(InstanceGuard),
    Reopened,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenRequest {
    Show,
    Settings,
}

/// Native handles are owned, Send + Sync, and closed automatically on drop.
/// The lifetime mutex is deliberately *not owned by a thread*: its named-object
/// existence elects the primary, so dropping an Arc on a different thread is safe.
#[derive(Debug)]
pub struct InstanceGuard {
    #[cfg(windows)]
    native: native::NativeGuard,
}

impl InstanceGuard {
    /// Uses the canonical profile directory (`POPCHAT_DATA_DIR` when set).
    /// `--settings` is forwarded; all other secondary launches request SHOW.
    pub fn acquire() -> anyhow::Result<InstanceLaunch> {
        let request = if std::env::args_os().any(|arg| arg == "--settings") {
            ReopenRequest::Settings
        } else {
            ReopenRequest::Show
        };
        Self::acquire_with_request(request)
    }

    /// Explicit-request variant for callers that already parsed command arguments.
    pub fn acquire_with_request(request: ReopenRequest) -> anyhow::Result<InstanceLaunch> {
        #[cfg(windows)]
        {
            let profile = crate::chat::store::app_data_dir();
            std::fs::create_dir_all(&profile)?;
            let namespace = native::profile_namespace(&profile)?;
            native::acquire_named(&namespace, request)
        }
        #[cfg(not(windows))]
        {
            let _ = request;
            anyhow::bail!("單一執行個體僅支援 Windows")
        }
    }

    /// Nonblocking and safe to call while hidden. Repeated SHOW requests coalesce;
    /// Settings has priority if both event kinds are pending at the same poll.
    /// The caller must show/activate, not toggle, and dismiss any first-run hint.
    pub fn take_request(&self) -> Option<ReopenRequest> {
        #[cfg(windows)]
        {
            self.native.take_request()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// Compatibility helper when the caller only needs SHOW. Consumes Settings
    /// requests too; prefer `take_request` to preserve explicit Settings intent.
    pub fn take_reopen_request(&self) -> bool {
        self.take_request().is_some()
    }
}

#[cfg(windows)]
mod native {
    use super::{InstanceGuard, InstanceLaunch, ReopenRequest};
    use anyhow::Context;
    use sha2::{Digest, Sha256};
    use std::ffi::c_void;
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use std::path::Path;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_ABANDONED: u32 = 0x80;
    const INFINITE: u32 = u32::MAX;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *const c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        fn CreateEventW(
            attributes: *const c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> *mut c_void;
        fn GetLastError() -> u32;
        fn SetEvent(event: *mut c_void) -> i32;
        fn ResetEvent(event: *mut c_void) -> i32;
        fn ReleaseMutex(mutex: *mut c_void) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    }

    /// Canonicalization unifies relative paths, `..`, junctions and extended path
    /// spellings. Lowercase Unicode paths without discarding unpaired UTF-16.
    pub(super) fn profile_namespace(profile: &Path) -> anyhow::Result<String> {
        let path = std::fs::canonicalize(profile).context("解析啟動設定檔路徑失敗")?;
        let mut digest = Sha256::new();
        for character in char::decode_utf16(path.as_os_str().encode_wide()) {
            match character {
                Ok(character) => {
                    for lower in character.to_lowercase() {
                        let mut units = [0; 2];
                        for unit in lower.encode_utf16(&mut units) {
                            digest.update(unit.to_le_bytes());
                        }
                    }
                }
                Err(error) => digest.update(error.unpaired_surrogate().to_le_bytes()),
            }
        }
        // Local scopes to the interactive Windows session; default object ACLs
        // retain the creator user's protection. No credentials enter the name.
        Ok(format!("Local\\PopChat.Profile.{:x}", digest.finalize()))
    }

    fn wide_name(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn owned_handle(handle: *mut c_void) -> std::io::Result<OwnedHandle> {
        if handle.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: successful Create* returns a new uniquely owned handle;
            // OwnedHandle closes it exactly once, including error unwinding.
            Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    fn create_event(name: &str) -> anyhow::Result<OwnedHandle> {
        let name = wide_name(name);
        // SAFETY: null attributes select the default ACL, name is NUL-terminated
        // and lives through the call. Auto-reset gives nonblocking consume-on-poll.
        // Initial state is ignored for existing events. Only a newly elected
        // primary may reset events, while holding the initialization lock.
        owned_handle(unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) })
            .context("建立重新開啟事件失敗")
    }

    fn create_mutex(name: &str) -> anyhow::Result<(OwnedHandle, bool)> {
        let name = wide_name(name);
        // SAFETY: valid NUL-terminated name, no thread ownership requested.
        let raw = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        // Capture immediately: later Win32 calls must not overwrite last-error.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok((
            owned_handle(raw).context("建立單一執行個體鎖失敗")?,
            existed,
        ))
    }

    fn signal(event: &OwnedHandle) -> anyhow::Result<()> {
        // SAFETY: handle is a live event owned for the duration of this call.
        if unsafe { SetEvent(event.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error()).context("通知執行中的 PopChat 失敗");
        }
        Ok(())
    }

    fn take(event: &OwnedHandle) -> bool {
        // SAFETY: live waitable event; zero timeout never blocks the UI driver.
        unsafe { WaitForSingleObject(event.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }

    /// Short, thread-owned initialization mutex, unlike the lifetime marker.
    /// !Send/!Sync ensures ReleaseMutex always runs on the acquiring thread.
    struct InitializationLock {
        handle: OwnedHandle,
        _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
    }

    impl InitializationLock {
        fn acquire(namespace: &str) -> anyhow::Result<Self> {
            let (handle, _) = create_mutex(&format!("{namespace}.Initialize"))?;
            // SAFETY: live mutex handle. Windows releases thread ownership after
            // a crashed initializer; WAIT_ABANDONED transfers ownership to us.
            match unsafe { WaitForSingleObject(handle.as_raw_handle(), INFINITE) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self {
                    handle,
                    _thread_bound: std::marker::PhantomData,
                }),
                _ => Err(std::io::Error::last_os_error()).context("等待啟動初始化鎖失敗"),
            }
        }
    }

    impl Drop for InitializationLock {
        fn drop(&mut self) {
            // SAFETY: this !Send guard owns the mutex on the current thread.
            unsafe { ReleaseMutex(self.handle.as_raw_handle()) };
        }
    }

    fn reset(event: &OwnedHandle) -> anyhow::Result<()> {
        // SAFETY: live event, called only inside the initialization lock before
        // this generation's secondary processes are allowed to signal anything.
        if unsafe { ResetEvent(event.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error()).context("初始化重新開啟事件失敗");
        }
        Ok(())
    }

    #[derive(Debug)]
    pub(super) struct NativeGuard {
        show: OwnedHandle,
        settings: OwnedHandle,
        // Drop events before the mutex, retaining the primary election marker
        // throughout event cleanup. No ReleaseMutex (we never took ownership).
        _mutex: OwnedHandle,
    }

    impl NativeGuard {
        pub(super) fn take_request(&self) -> Option<ReopenRequest> {
            if take(&self.settings) {
                let _ = take(&self.show);
                Some(ReopenRequest::Settings)
            } else if take(&self.show) {
                Some(ReopenRequest::Show)
            } else {
                None
            }
        }
    }

    pub(super) fn acquire_named(
        namespace: &str,
        request: ReopenRequest,
    ) -> anyhow::Result<InstanceLaunch> {
        // Serialize election + event initialization + secondary signaling. A
        // secondary arriving before the events exist waits here, not in an
        // OpenEvent retry loop. A fresh primary can safely clear orphaned events
        // without erasing requests from this generation. The lock is released
        // before returning, and never lives in the Send + Sync application guard.
        let _initializing = InitializationLock::acquire(namespace)?;
        let (mutex, existed) = create_mutex(&format!("{namespace}.Instance"))?;
        let show = create_event(&format!("{namespace}.Show"))?;
        let settings = create_event(&format!("{namespace}.Settings"))?;
        if existed {
            signal(match request {
                ReopenRequest::Show => &show,
                ReopenRequest::Settings => &settings,
            })?;
            Ok(InstanceLaunch::Reopened)
        } else {
            reset(&show)?;
            reset(&settings)?;
            Ok(InstanceLaunch::Primary(InstanceGuard {
                native: NativeGuard {
                    show,
                    settings,
                    _mutex: mutex,
                },
            }))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "isolated child-process fixture, invoked by native_secondary_process_signals_primary"]
        fn isolated_secondary_process_helper() {
            let Ok(namespace) = std::env::var("POPCHAT_INSTANCE_TEST_NAMESPACE") else {
                return;
            };
            assert!(namespace.starts_with("Local\\PopChat.Test."));
            let request =
                if std::env::var("POPCHAT_INSTANCE_TEST_REQUEST").as_deref() == Ok("settings") {
                    ReopenRequest::Settings
                } else {
                    ReopenRequest::Show
                };
            assert!(matches!(
                acquire_named(&namespace, request).unwrap(),
                InstanceLaunch::Reopened
            ));
        }

        #[test]
        fn native_secondary_process_signals_primary() {
            let namespace = namespace();
            let guard = primary(&namespace);
            for (value, expected) in [
                ("show", ReopenRequest::Show),
                ("show", ReopenRequest::Show),
                ("settings", ReopenRequest::Settings),
            ] {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "instance::native::tests::isolated_secondary_process_helper",
                        "--ignored",
                    ])
                    .env("POPCHAT_INSTANCE_TEST_NAMESPACE", &namespace)
                    .env("POPCHAT_INSTANCE_TEST_REQUEST", value)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    if child.try_wait().unwrap().is_some() {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("isolated secondary process failed to terminate");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(guard.take_request(), Some(expected));
                assert_eq!(guard.take_request(), None);
            }
        }

        fn namespace() -> String {
            // Never create/open the production profile's named objects.
            format!("Local\\PopChat.Test.{}", uuid::Uuid::new_v4().simple())
        }

        fn primary(namespace: &str) -> InstanceGuard {
            match acquire_named(namespace, ReopenRequest::Show).unwrap() {
                InstanceLaunch::Primary(guard) => guard,
                InstanceLaunch::Reopened => panic!("expected unique test primary"),
            }
        }

        #[test]
        fn secondary_reopens_without_toggling_and_settings_has_priority() {
            let namespace = namespace();
            let guard = primary(&namespace);
            assert_eq!(guard.take_request(), None);
            for _ in 0..2 {
                assert!(matches!(
                    acquire_named(&namespace, ReopenRequest::Show).unwrap(),
                    InstanceLaunch::Reopened
                ));
            }
            assert_eq!(guard.take_request(), Some(ReopenRequest::Show));
            assert_eq!(guard.take_request(), None);
            acquire_named(&namespace, ReopenRequest::Show).unwrap();
            acquire_named(&namespace, ReopenRequest::Settings).unwrap();
            assert_eq!(guard.take_request(), Some(ReopenRequest::Settings));
            assert_eq!(guard.take_request(), None);
            acquire_named(&namespace, ReopenRequest::Show).unwrap();
            assert!(guard.take_reopen_request());
            assert!(!guard.take_reopen_request());
        }

        #[test]
        fn request_before_primary_finishes_initialization_is_not_reset() {
            let namespace = namespace();
            // Elected primary pauses before creating either event. The second
            // launch can arrive now, but must wait for initialization to finish.
            let initializing = InitializationLock::acquire(&namespace).unwrap();
            let (mutex, existed) = create_mutex(&format!("{namespace}.Instance")).unwrap();
            assert!(!existed);
            let second_namespace = namespace.clone();
            let (started, ready) = std::sync::mpsc::channel();
            let secondary = std::thread::spawn(move || {
                started.send(()).unwrap();
                assert!(matches!(
                    acquire_named(&second_namespace, ReopenRequest::Show).unwrap(),
                    InstanceLaunch::Reopened
                ));
            });
            ready.recv().unwrap();
            let show = create_event(&format!("{namespace}.Show")).unwrap();
            let settings = create_event(&format!("{namespace}.Settings")).unwrap();
            reset(&show).unwrap();
            reset(&settings).unwrap();
            let guard = InstanceGuard {
                native: NativeGuard {
                    show,
                    settings,
                    _mutex: mutex,
                },
            };
            drop(initializing);
            secondary.join().unwrap();
            assert_eq!(guard.take_request(), Some(ReopenRequest::Show));
            assert_eq!(guard.take_request(), None);
        }

        #[test]
        fn fresh_primary_clears_preexisting_orphaned_event_signals() {
            let namespace = namespace();
            let show = create_event(&format!("{namespace}.Show")).unwrap();
            let settings = create_event(&format!("{namespace}.Settings")).unwrap();
            signal(&show).unwrap();
            signal(&settings).unwrap();
            let guard = primary(&namespace);
            assert_eq!(guard.take_request(), None);
            acquire_named(&namespace, ReopenRequest::Show).unwrap();
            assert_eq!(guard.take_request(), Some(ReopenRequest::Show));
        }

        #[test]
        fn simultaneous_launches_elect_exactly_one_primary() {
            let namespace = namespace();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let launches: Vec<_> = (0..8)
                .map(|_| {
                    let namespace = namespace.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        acquire_named(&namespace, ReopenRequest::Show).unwrap()
                    })
                })
                .collect();
            // Keep every returned guard alive until all acquisitions complete.
            let launches: Vec<_> = launches
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            let primaries: Vec<_> = launches
                .iter()
                .filter_map(|launch| match launch {
                    InstanceLaunch::Primary(guard) => Some(guard),
                    InstanceLaunch::Reopened => None,
                })
                .collect();
            assert_eq!(primaries.len(), 1);
            assert_eq!(primaries[0].take_request(), Some(ReopenRequest::Show));
            assert_eq!(primaries[0].take_request(), None);
        }

        #[test]
        fn separate_namespaces_and_drop_release_all_native_state() {
            let first_name = namespace();
            let second_name = namespace();
            let first = primary(&first_name);
            let second = primary(&second_name);
            acquire_named(&first_name, ReopenRequest::Settings).unwrap();
            assert_eq!(second.take_request(), None);
            // Leave a pending event, then close all handles: next primary must
            // receive fresh nonsignaled events rather than stale reopen state.
            drop(first);
            let replacement = primary(&first_name);
            assert_eq!(replacement.take_request(), None);
        }

        #[test]
        fn guard_is_send_sync_and_can_poll_and_drop_on_a_different_thread() {
            fn assert_send_sync<T: Send + Sync>() {}
            assert_send_sync::<InstanceGuard>();
            let namespace = namespace();
            let guard = std::sync::Arc::new(primary(&namespace));
            acquire_named(&namespace, ReopenRequest::Show).unwrap();
            std::thread::spawn(move || {
                assert_eq!(guard.take_request(), Some(ReopenRequest::Show));
            })
            .join()
            .unwrap();
            assert_eq!(primary(&namespace).take_request(), None);
        }

        #[test]
        fn profile_identity_is_stable_and_isolated_without_environment_changes() {
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join("one");
            let second = root.path().join("two");
            std::fs::create_dir_all(&first).unwrap();
            std::fs::create_dir_all(&second).unwrap();
            assert_eq!(
                profile_namespace(&first).unwrap(),
                profile_namespace(&first.join(".")).unwrap()
            );
            assert_ne!(
                profile_namespace(&first).unwrap(),
                profile_namespace(&second).unwrap()
            );
        }
    }
}
