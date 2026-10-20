//! C ABI of the Oxychloride OCL engine.
//!
//! The interface is deliberately tiny: create a session, send it JSON-RPC requests as
//! strings, get JSON responses back. Every method of the JSON-RPC protocol (see
//! `docs/PROTOCOL.md`) is available, so bindings for other languages (Java via JNA or the
//! FFM API, Python via ctypes, C#, ...) only need these few functions. The C declarations
//! are in `include/oxychloride.h`.
//!
//! Strings returned by this library must be released with [`oxy_string_free`]. Sessions are
//! not thread-safe: use one session per thread, or synchronize externally.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use oxychloride::rpc::Server;

/// An engine session holding loaded metamodels, models and OCL documents.
pub struct OxySession {
    server: Server,
}

fn to_c_string(s: String) -> *mut c_char {
    // Interior NUL bytes cannot occur in serialized JSON (they are escaped), but be safe.
    CString::new(s.replace('\0', "\\u0000")).map_or(ptr::null_mut(), CString::into_raw)
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32603, "message": message } }).to_string()
}

/// Creates a new, empty session. Release it with [`oxy_session_free`].
#[unsafe(no_mangle)]
pub extern "C" fn oxy_session_new() -> *mut OxySession {
    Box::into_raw(Box::new(OxySession { server: Server::new() }))
}

/// Releases a session created by [`oxy_session_new`]. Passing NULL is allowed.
///
/// # Safety
/// `session` must be NULL or a pointer returned by [`oxy_session_new`] that has not been
/// freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oxy_session_free(session: *mut OxySession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// Handles a JSON-RPC 2.0 request (or batch) given as a NUL-terminated UTF-8 string and
/// returns the JSON response. Returns NULL for notifications (requests without an `id`).
///
/// # Safety
/// `session` must be a valid session and `request` a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oxy_session_request(session: *mut OxySession, request: *const c_char) -> *mut c_char {
    if session.is_null() || request.is_null() {
        return to_c_string(error_json("null argument"));
    }
    let session = unsafe { &mut *session };
    let request = match unsafe { CStr::from_ptr(request) }.to_str() {
        Ok(r) => r,
        Err(_) => return to_c_string(error_json("request is not valid UTF-8")),
    };
    match catch_unwind(AssertUnwindSafe(|| session.server.handle_str(request))) {
        Ok(Some(response)) => to_c_string(response),
        Ok(None) => ptr::null_mut(),
        Err(_) => to_c_string(error_json("internal error (panic) while handling the request")),
    }
}

/// Calls a method directly. `params` is a JSON value (or NULL for no parameters). Returns
/// `{"result": ...}` on success or `{"error": {"code": ..., "message": ...}}` on failure.
///
/// # Safety
/// `session` must be a valid session; `method` a valid NUL-terminated string; `params`
/// NULL or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oxy_session_call(
    session: *mut OxySession,
    method: *const c_char,
    params: *const c_char,
) -> *mut c_char {
    if session.is_null() || method.is_null() {
        return to_c_string(error_json("null argument"));
    }
    let session = unsafe { &mut *session };
    let Ok(method) = unsafe { CStr::from_ptr(method) }.to_str() else {
        return to_c_string(error_json("method is not valid UTF-8"));
    };
    let params = if params.is_null() {
        serde_json::Value::Null
    } else {
        match unsafe { CStr::from_ptr(params) }.to_str().map(serde_json::from_str::<serde_json::Value>) {
            Ok(Ok(v)) => v,
            _ => {
                return to_c_string(
                    serde_json::json!({ "error": { "code": -32700, "message": "params are not valid JSON" } })
                        .to_string(),
                );
            }
        }
    };
    let result = catch_unwind(AssertUnwindSafe(|| session.server.call(method, params)));
    let response = match result {
        Ok(Ok(v)) => serde_json::json!({ "result": v }),
        Ok(Err(e)) => serde_json::json!({ "error": { "code": e.code, "message": e.message } }),
        Err(_) => serde_json::json!({ "error": { "code": -32603, "message": "internal error (panic)" } }),
    };
    to_c_string(response.to_string())
}

/// Releases a string returned by this library. Passing NULL is allowed.
///
/// # Safety
/// `s` must be NULL or a string returned by this library that has not been freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oxy_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The library version as a static NUL-terminated string (do not free).
#[unsafe(no_mangle)]
pub extern "C" fn oxy_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(session: *mut OxySession, method: &str, params: &str) -> serde_json::Value {
        let m = CString::new(method).unwrap();
        let p = CString::new(params).unwrap();
        unsafe {
            let out = oxy_session_call(session, m.as_ptr(), p.as_ptr());
            let v = serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap();
            oxy_string_free(out);
            v
        }
    }

    #[test]
    fn round_trip() {
        let s = oxy_session_new();
        let v = call(s, "evaluate", r#"{"expression": "Set{1, 2, 3}->sum()"}"#);
        assert_eq!(v["result"]["display"], "6");
        let v = call(s, "nope", "{}");
        assert_eq!(v["error"]["code"], -32601);
        let req = CString::new(r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#).unwrap();
        unsafe {
            let out = oxy_session_request(s, req.as_ptr());
            let v: serde_json::Value = serde_json::from_str(CStr::from_ptr(out).to_str().unwrap()).unwrap();
            assert_eq!(v["id"], 7);
            assert_eq!(v["result"]["name"], "oxychloride");
            oxy_string_free(out);
            oxy_session_free(s);
        }
    }
}
