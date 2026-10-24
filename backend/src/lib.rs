//! C ABI for the C++ UI and the shared library entry point.

mod api;
mod config;
mod games;
mod icons;
mod install;
mod launch;
mod lists;
mod logs;
mod paths;
mod protons;
mod saved;
mod settings;
mod steam_art;

pub use api::{handle, Ctx};
pub use launch::clean_proton_env;
pub use protons::steam_root;

use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

pub struct MpBackend {
    ctx: Mutex<Ctx>,
}

#[no_mangle]
pub extern "C" fn mp_backend_new() -> *mut MpBackend {
    Box::into_raw(Box::new(MpBackend {
        ctx: Mutex::new(Ctx::new()),
    }))
}

/// # Safety
/// `backend` must be a pointer returned by [`mp_backend_new`] that has not
/// been freed yet.
#[no_mangle]
pub unsafe extern "C" fn mp_backend_free(backend: *mut MpBackend) {
    if !backend.is_null() {
        unsafe {
            drop(Box::from_raw(backend));
        }
    }
}

/// # Safety
/// `backend` must come from [`mp_backend_new`] and `request` must be a valid
/// NUL-terminated UTF-8 string. The returned pointer is owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn mp_backend_call(
    backend: *mut MpBackend,
    request: *const c_char,
) -> *mut c_char {
    let reply = if backend.is_null() || request.is_null() {
        "{\"ok\":false,\"error\":\"null backend or request\"}".to_owned()
    } else {
        let raw = unsafe { CStr::from_ptr(request) }.to_string_lossy();
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) => match unsafe { &*backend }.ctx.lock() {
                Ok(mut ctx) => handle(&mut ctx, &value).to_string(),
                Err(_) => "{\"ok\":false,\"error\":\"backend lock poisoned\"}".to_owned(),
            },
            Err(err) => format!("{{\"ok\":false,\"error\":\"bad JSON: {err}\"}}"),
        }
    };
    CString::new(reply)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"NUL in reply\"}").unwrap())
        .into_raw()
}

/// # Safety
/// `backend` must be a pointer returned by [`mp_backend_new`] that has not
/// been freed yet.
/// # Safety
/// `text` must be a pointer returned by [`mp_backend_call`] and not freed yet.
#[no_mangle]
pub unsafe extern "C" fn mp_backend_free_string(text: *mut c_char) {
    if !text.is_null() {
        unsafe {
            drop(CString::from_raw(text));
        }
    }
}
