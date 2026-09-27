//! Native extension ABI v1: C functions and owned UTF-8 JSON buffers, never Rust objects.
use anyhow::{Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::{CStr, CString, c_char};

pub const ABI_VERSION: u32 = 1;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub result: Option<Value>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub abi_version: u32,
    pub name: String,
    pub version: String,
    pub methods: Vec<String>,
    #[serde(default)]
    pub schemes: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}
pub fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
pub fn decode(value: &Value) -> Result<Vec<u8>> {
    let s = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("expected base64 string"))?;
    if s.len() > MAX_BYTES * 2 {
        bail!("payload too large");
    }
    let data = base64::engine::general_purpose::STANDARD.decode(s)?;
    if data.len() > MAX_BYTES {
        bail!("payload too large");
    }
    Ok(data)
}
pub fn string<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string parameter {key}"))
}

/// Runs a dispatcher with panic containment and extension-owned return allocation.
///
/// # Safety
/// `input` must point to a live NUL-terminated UTF-8 JSON string for the duration of this call.
pub unsafe fn invoke(input: *const c_char, handler: fn(Request) -> Result<Value>) -> *mut c_char {
    let outcome = std::panic::catch_unwind(|| -> Result<Value> {
        if input.is_null() {
            bail!("null request");
        }
        // SAFETY: required by caller contract, no pointer escapes this function.
        let bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
        if bytes.len() > MAX_BYTES * 2 {
            bail!("request too large");
        }
        handler(serde_json::from_slice(bytes)?)
    });
    let response = match outcome {
        Ok(Ok(value)) => Response {
            result: Some(value),
            error: None,
        },
        Ok(Err(e)) => Response {
            result: None,
            error: Some(format!("{e:#}")),
        },
        Err(_) => Response {
            result: None,
            error: Some("extension panicked".into()),
        },
    };
    CString::new(serde_json::to_vec(&response).expect("JSON serialization"))
        .expect("JSON contains no NUL bytes")
        .into_raw()
}
/// Releases a response allocated by this library.
///
/// # Safety
/// `ptr` must be null or an unfreed pointer returned by `invoke` in the same library.
pub unsafe fn release(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

#[macro_export]
macro_rules! export_extension {
    ($handler:path) => {
        /// Returns the supported C ABI version.
        #[unsafe(no_mangle)]
        pub extern "C" fn syncer_extension_abi_version() -> u32 {
            $crate::ABI_VERSION
        }
        /// Executes a UTF-8 JSON request.
        /// # Safety
        /// Input must be a valid, live NUL-terminated C string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn syncer_extension_invoke(
            input: *const ::std::ffi::c_char,
        ) -> *mut ::std::ffi::c_char {
            unsafe { $crate::invoke(input, $handler) }
        }
        /// Releases a response allocated by this extension.
        /// # Safety
        /// Pointer must have been returned by this extension, and not previously freed.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn syncer_extension_free(ptr: *mut ::std::ffi::c_char) {
            unsafe { $crate::release(ptr) }
        }
    };
}
