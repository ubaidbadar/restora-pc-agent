#[cfg(windows)]
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Credentials used only by the native agent. Never returned over localhost.
#[derive(Clone)]
pub struct Config {
    pub api: String,
    pub token: String,
    pub branch_id: Option<String>,
}

/// Legacy installations contain "token". New Windows installations contain
/// only "protectedToken", encrypted to the Windows user/machine with DPAPI.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    api: String,
    #[serde(default)]
    branch_id: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    protected_token: Option<String>,
}

/// %APPDATA%\restora on Windows (~/.config/restora elsewhere).
pub fn folder() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let folder = base.unwrap_or_else(|| PathBuf::from(".")).join("restora");
    let _ = std::fs::create_dir_all(&folder);
    folder
}

pub fn load() -> Option<Config> {
    let stored: StoredConfig =
        serde_json::from_slice(&std::fs::read(folder().join("agent.json")).ok()?).ok()?;
    let legacy = stored.token.is_some();
    let token = match (stored.protected_token, stored.token) {
        (Some(encoded), _) => {
            #[cfg(windows)]
            {
                let ciphertext = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()?;
                String::from_utf8(dpapi::unprotect(&ciphertext).ok()?).ok()?
            }
            #[cfg(not(windows))]
            {
                let _ = encoded;
                return None;
            }
        }
        (None, Some(plaintext)) => plaintext,
        (None, None) => return None,
    };
    let config = Config {
        api: stored.api,
        token,
        branch_id: stored.branch_id,
    };
    #[cfg(windows)]
    if legacy {
        // Automatically migrate the old plaintext file on first successful read.
        // If Windows denies DPAPI, keep the original file and retry migration later.
        if let Err(error) = save(&config) {
            tracing::warn!("could not protect legacy agent token: {error}");
        }
    }
    #[cfg(not(windows))]
    let _ = legacy;
    Some(config)
}

pub fn save(config: &Config) -> std::io::Result<()> {
    #[cfg(windows)]
    let stored = StoredConfig {
        api: config.api.clone(),
        branch_id: config.branch_id.clone(),
        token: None,
        protected_token: Some(
            base64::engine::general_purpose::STANDARD
                .encode(dpapi::protect(config.token.as_bytes())?),
        ),
    };
    #[cfg(not(windows))]
    let stored = StoredConfig {
        api: config.api.clone(),
        branch_id: config.branch_id.clone(),
        token: Some(config.token.clone()),
        protected_token: None,
    };
    std::fs::write(
        folder().join("agent.json"),
        serde_json::to_vec_pretty(&stored)?,
    )
}

pub fn clear() {
    let _ = std::fs::remove_file(folder().join("agent.json"));
}

/// A server-side removal is deliberate. Keep this marker across restarts so the app does not silently re-pair.
pub fn require_reconnect() {
    let _ = std::fs::write(folder().join("reconnect-required"), "");
}

pub fn reconnect_required() -> bool {
    folder().join("reconnect-required").exists()
}

pub fn clear_reconnect_required() {
    let _ = std::fs::remove_file(folder().join("reconnect-required"));
}

#[cfg(windows)]
mod dpapi {
    use std::{ffi::c_void, io, ptr};

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "Crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn LocalFree(handle: *mut c_void) -> *mut c_void;
    }

    const UI_FORBIDDEN: u32 = 0x1;

    fn transform(input: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
        if input.len() > u32::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "token is too large",
            ));
        }
        let mut data = input.to_vec();
        let blob = DataBlob {
            cb_data: data.len() as u32,
            pb_data: data.as_mut_ptr(),
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        // DPAPI encrypts for the current Windows user, without exporting a key.
        // The returned allocation is owned by LocalFree (Windows API contract).
        let ok = unsafe {
            if encrypt {
                CryptProtectData(
                    &blob,
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &blob,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    UI_FORBIDDEN,
                    &mut output,
                )
            }
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result =
            unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();
        unsafe {
            LocalFree(output.pb_data.cast());
        }
        Ok(result)
    }

    pub fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
        transform(bytes, true)
    }

    pub fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
        transform(bytes, false)
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn dpapi_roundtrip_restores_token_without_plaintext_on_disk() {
        let token = b"restora-pairing-secret\x00with-bytes";
        let encrypted = super::dpapi::protect(token).unwrap();
        assert_ne!(encrypted, token);
        assert_eq!(super::dpapi::unprotect(&encrypted).unwrap(), token);
    }
}
