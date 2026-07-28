use serde::Serialize;
use std::path::Path;
use std::process::Command;

const LOGIN_FRAMEWORK: &[u8] =
    b"/System/Library/PrivateFrameworks/login.framework/Versions/A/login\0";
const LOCK_SYMBOL: &[u8] = b"SACLockScreenImmediate\0";
const SCREENSAVER_APP: &str = "/System/Library/CoreServices/ScreenSaverEngine.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockBackend {
    /// Immediate, real lock. Preferred.
    PrivateApi,
    /// Starts the screen saver. Only locks if the user enabled
    /// "require password after screen saver begins" — which we cannot verify,
    /// because com.apple.screensaver askForPassword is no longer readable.
    ScreenSaver,
    Unavailable,
}

pub trait ScreenLocker: Send + Sync {
    fn lock(&self) -> Result<(), String>;
    fn backend(&self) -> LockBackend;
}

type LockFn = unsafe extern "C" fn() -> i32;

fn resolve_lock_symbol() -> Option<LockFn> {
    unsafe {
        let handle = libc::dlopen(LOGIN_FRAMEWORK.as_ptr() as *const _, libc::RTLD_LAZY);
        if handle.is_null() {
            return None;
        }
        let sym = libc::dlsym(handle, LOCK_SYMBOL.as_ptr() as *const _);
        if sym.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut libc::c_void, LockFn>(sym))
        }
    }
}

pub struct MacScreenLocker {
    backend: LockBackend,
}

impl MacScreenLocker {
    pub fn detect() -> Self {
        let backend = if resolve_lock_symbol().is_some() {
            LockBackend::PrivateApi
        } else if Path::new(SCREENSAVER_APP).exists() {
            LockBackend::ScreenSaver
        } else {
            LockBackend::Unavailable
        };
        Self { backend }
    }
}

impl ScreenLocker for MacScreenLocker {
    fn backend(&self) -> LockBackend {
        self.backend
    }

    fn lock(&self) -> Result<(), String> {
        match self.backend {
            LockBackend::PrivateApi => {
                let f = resolve_lock_symbol().ok_or("lock symbol disappeared at runtime")?;
                let rc = unsafe { f() };
                if rc == 0 {
                    Ok(())
                } else {
                    Err(format!("SACLockScreenImmediate returned {rc}"))
                }
            }
            LockBackend::ScreenSaver => Command::new("/usr/bin/open")
                .arg("-a")
                .arg(SCREENSAVER_APP)
                .status()
                .map_err(|e| format!("failed to start screen saver: {e}"))
                .and_then(|s| if s.success() { Ok(()) } else { Err("screen saver failed".into()) }),
            LockBackend::Unavailable => Err("no screen lock mechanism available".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_usable_backend_on_this_machine() {
        let locker = MacScreenLocker::detect();
        assert_ne!(locker.backend(), LockBackend::Unavailable);
    }

    #[test]
    fn prefers_the_private_api_when_the_symbol_resolves() {
        // Verified present on macOS 26. If this ever fails, the fallback
        // path became load-bearing and the UI warning matters.
        assert_eq!(MacScreenLocker::detect().backend(), LockBackend::PrivateApi);
    }
}
