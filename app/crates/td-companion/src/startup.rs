use crate::api::{CommonError, Ctx, Endpoint, Feature, NoQuery, Registration, Req};
use axum::http::Method;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
pub trait Startup: Send + Sync {
    fn get(&self) -> Result<bool, CommonError>;
    fn set(&self, enabled: bool) -> Result<(), CommonError>;
    /// Apply the first packaged-start default without reversing an OS disable.
    fn initialize_default(&self) -> Result<(), CommonError> {
        self.set(true)
    }
}

// Explorer's undocumented StartupApproved format is a little-endian DWORD
// state followed by an eight-byte FILETIME. Recognise only the observed enabled
// states (2 and 6); disabled (3/7), unknown and malformed records stay off.
// Absence means Explorer has not blocked the Run registration. Do not infer
// enabled state from a zero timestamp: a disabled record can have one too.
#[cfg(any(windows, test))]
fn startup_approved_allows_start(value: Option<&[u8]>) -> bool {
    match value {
        None => true,
        Some(bytes) if bytes.len() == 12 => {
            matches!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 2 | 6)
        }
        Some(_) => false,
    }
}
#[derive(Default)]
pub struct FakeStartup(pub std::sync::atomic::AtomicBool);
impl Startup for FakeStartup {
    fn get(&self) -> Result<bool, CommonError> {
        Ok(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
    fn set(&self, enabled: bool) -> Result<(), CommonError> {
        self.0.store(enabled, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
pub struct NativeStartup;
#[cfg(windows)]
mod native {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{Foundation::*, System::Registry::*};
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APPROVED: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
    const NAME: &str = "Open TouchDeck";
    fn approved() -> Result<bool, CommonError> {
        let mut bytes = [0; 12];
        let mut size = bytes.len() as u32;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(APPROVED).as_ptr(),
                wide(NAME).as_ptr(),
                RRF_RT_REG_BINARY,
                null_mut(),
                bytes.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match result {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(startup_approved_allows_start(None)),
            ERROR_SUCCESS => Ok(startup_approved_allows_start(Some(&bytes[..size as usize]))),
            ERROR_MORE_DATA | ERROR_UNSUPPORTED_TYPE => Ok(false),
            _ => Err(CommonError::Internal),
        }
    }
    impl Startup for NativeStartup {
        fn get(&self) -> Result<bool, CommonError> {
            let mut size = 0;
            let result = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    wide(KEY).as_ptr(),
                    wide(NAME).as_ptr(),
                    RRF_RT_REG_SZ,
                    null_mut(),
                    null_mut(),
                    &mut size,
                )
            };
            match result {
                ERROR_SUCCESS => approved(),
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(false),
                _ => Err(CommonError::Internal),
            }
        }
        fn set(&self, enabled: bool) -> Result<(), CommonError> {
            set_run(enabled)?;
            if enabled {
                // Only an explicit user opt-in clears Explorer's veto. The
                // first-start default must never pass through this path.
                let result = unsafe {
                    RegDeleteKeyValueW(
                        HKEY_CURRENT_USER,
                        wide(APPROVED).as_ptr(),
                        wide(NAME).as_ptr(),
                    )
                };
                if !matches!(
                    result,
                    ERROR_SUCCESS | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND
                ) {
                    return Err(CommonError::Internal);
                }
            }
            Ok(())
        }
        fn initialize_default(&self) -> Result<(), CommonError> {
            if approved()? {
                set_run(true)?;
            }
            Ok(())
        }
    }
    fn set_run(enabled: bool) -> Result<(), CommonError> {
        let command = if enabled {
            let exe = std::env::current_exe().map_err(|_| CommonError::Internal)?;
            wide(&format!("\"{}\" --packaged", exe.display()))
        } else {
            Vec::new()
        };
        let mut key = null_mut();
        let result = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(KEY).as_ptr(),
                0,
                null(),
                0,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        };
        if result != ERROR_SUCCESS {
            return Err(CommonError::Internal);
        }
        let result = if enabled {
            unsafe {
                RegSetValueExW(
                    key,
                    wide(NAME).as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            unsafe { RegDeleteValueW(key, wide(NAME).as_ptr()) }
        };
        unsafe {
            RegCloseKey(key);
        }
        if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(CommonError::Internal)
        }
    }
}
#[cfg(not(windows))]
impl Startup for NativeStartup {
    fn get(&self) -> Result<bool, CommonError> {
        Ok(false)
    }
    fn set(&self, enabled: bool) -> Result<(), CommonError> {
        if enabled {
            Err(CommonError::Internal)
        } else {
            Ok(())
        }
    }
}
#[derive(Serialize, Deserialize, TS)]
pub struct StartupSetting {
    pub at_sign_in: bool,
}
pub struct Get;
impl Endpoint for Get {
    const NAME: &'static str = "system.startup_get";
    const METHOD: Method = Method::GET;
    const PATH: &'static str = "/system/startup";
    type Path = ();
    type Query = NoQuery;
    type Body = ();
    type Ok = StartupSetting;
    type Error = CommonError;
    async fn handle(ctx: Ctx, _: Req<Self>) -> Result<Self::Ok, Self::Error> {
        tokio::task::spawn_blocking(move || {
            ctx.desktop
                .startup
                .get()
                .map(|at_sign_in| StartupSetting { at_sign_in })
        })
        .await
        .map_err(|_| CommonError::Internal)?
    }
}
pub struct Put;
impl Endpoint for Put {
    const NAME: &'static str = "system.startup_put";
    const METHOD: Method = Method::PUT;
    const PATH: &'static str = "/system/startup";
    type Path = ();
    type Query = NoQuery;
    type Body = StartupSetting;
    type Ok = StartupSetting;
    type Error = CommonError;
    async fn handle(ctx: Ctx, req: Req<Self>) -> Result<Self::Ok, Self::Error> {
        tokio::task::spawn_blocking(move || {
            ctx.desktop.startup.set(req.body.at_sign_in)?;
            Ok(req.body)
        })
        .await
        .map_err(|_| CommonError::Internal)?
    }
}
fn feature() -> Feature {
    Feature::new("system").endpoint::<Get>().endpoint::<Put>()
}
inventory::submit! { Registration(feature) }

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: u32, timestamp: u64) -> [u8; 12] {
        let mut bytes = [0; 12];
        bytes[..4].copy_from_slice(&state.to_le_bytes());
        bytes[4..].copy_from_slice(&timestamp.to_le_bytes());
        bytes
    }

    #[test]
    fn absent_approval_allows_the_run_registration() {
        assert!(startup_approved_allows_start(None));
    }

    #[test]
    fn known_enabled_states_allow_start_regardless_of_timestamp() {
        for state in [2, 6] {
            for timestamp in [0, 0x01da_1234_5678_9abc] {
                assert!(startup_approved_allows_start(Some(&record(
                    state, timestamp
                ))));
            }
        }
    }

    #[test]
    fn disabled_states_never_allow_start_even_with_a_zero_timestamp() {
        for state in [3, 7] {
            for timestamp in [0, 0x01da_1234_5678_9abc] {
                assert!(!startup_approved_allows_start(Some(&record(
                    state, timestamp
                ))));
            }
        }
    }

    #[test]
    fn unknown_and_malformed_records_never_enable_startup() {
        for state in [0, 1, 4, 5, 8, 0x102, u32::MAX] {
            assert!(!startup_approved_allows_start(Some(&record(state, 0))));
        }
        let enabled = record(2, 0);
        for length in 0..12 {
            assert!(!startup_approved_allows_start(Some(&enabled[..length])));
        }
        assert!(!startup_approved_allows_start(Some(&[2; 13])));
    }
}
