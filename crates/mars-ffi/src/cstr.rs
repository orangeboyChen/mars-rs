//! Null-safe conversions from caller-owned C memory.
//!
//! Every `unsafe` in the crate funnels through this module so the audit surface
//! stays tiny: three functions, each with one `CStr`/raw-pointer read.
//!
//! The rule enforced here is the one the JNI layer already relied on
//! (`Java2C_Xlog.cc` null-checks every `jstring` before `ScopedJstring`):
//! a null pointer or a non-UTF-8 buffer degrades to `None` / `""`, never to a
//! panic that could unwind into C.

use std::ffi::{c_char, CStr};

/// Reads a NUL-terminated C string, returning `None` for a null pointer or
/// invalid UTF-8.
///
/// # Safety
///
/// `ptr` must either be null or point to a valid NUL-terminated array of
/// `c_char` that is not mutated while the returned borrow is alive. The caller
/// (the C/C++/JNI layer) owns that memory; this function never frees it.
pub unsafe fn ptr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null and — per the caller's contract — points to a
    // valid NUL-terminated string that outlives this call.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    // Invalid UTF-8 is reported as "absent" rather than panicking.
    cstr.to_str().ok()
}

/// [`ptr_to_str`] with `""` as the fallback, i.e. "treat null / invalid UTF-8 as
/// an empty string".
///
/// # Safety
///
/// Same as [`ptr_to_str`].
pub unsafe fn ptr_to_str_or_empty<'a>(ptr: *const c_char) -> &'a str {
    // SAFETY: forwarded to `ptr_to_str`, whose contract the caller upholds.
    unsafe { ptr_to_str(ptr) }.unwrap_or("")
}

/// Reads a NUL-terminated C string into an owned `PathBuf`, preserving bytes
/// that are not valid UTF-8.
///
/// Paths are bytes: `ptr_to_str_or_empty` turns a non-UTF-8 directory into
/// `""`, which `mars_xlog_open` then rejects with `EMPTY_LOG_DIR` — logging
/// silently off for a perfectly valid path. On unix the bytes are used as-is;
/// elsewhere they are converted lossily so the call still succeeds.
///
/// # Safety
///
/// Same as [`ptr_to_str`].
pub unsafe fn ptr_to_path_buf(ptr: *const c_char) -> std::path::PathBuf {
    if ptr.is_null() {
        return std::path::PathBuf::new();
    }
    // SAFETY: `ptr` is non-null and — per the caller's contract — points to a
    // valid NUL-terminated string that outlives this call.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes()))
    }
    #[cfg(not(unix))]
    {
        std::path::PathBuf::from(cstr.to_string_lossy().into_owned())
    }
}

/// Reads a single value through a possibly-null pointer.
///
/// # Safety
///
/// `ptr` must either be null or point to a properly aligned, initialised `T`
/// that is not mutated while the returned borrow is alive.
pub unsafe fn ptr_to_ref<'a, T>(ptr: *const T) -> Option<&'a T> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null, aligned and initialised per the caller's
    // contract, and the borrow does not outlive this call.
    Some(unsafe { &*ptr })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn null_is_none() {
        // SAFETY: null is explicitly allowed by the contract.
        assert_eq!(unsafe { ptr_to_str(std::ptr::null()) }, None);
        // SAFETY: null is explicitly allowed by the contract.
        assert_eq!(unsafe { ptr_to_str_or_empty(std::ptr::null()) }, "");
    }

    #[test]
    fn valid_string_is_read() {
        let owned = CString::new("hello xlog").unwrap();
        // SAFETY: `owned.as_ptr()` is a valid NUL-terminated string that
        // outlives this call.
        assert_eq!(unsafe { ptr_to_str(owned.as_ptr()) }, Some("hello xlog"));
        // SAFETY: same as above.
        assert_eq!(unsafe { ptr_to_str_or_empty(owned.as_ptr()) }, "hello xlog");
    }

    #[test]
    fn invalid_utf8_degrades_to_empty() {
        // `b"h\xff\0"` — a valid C string that is not valid UTF-8.
        let bytes = c"h\xff";
        // SAFETY: `bytes` is a valid NUL-terminated string.
        assert_eq!(unsafe { ptr_to_str(bytes.as_ptr()) }, None);
        // SAFETY: same as above.
        assert_eq!(unsafe { ptr_to_str_or_empty(bytes.as_ptr()) }, "");
    }

    #[test]
    fn empty_string_is_distinguishable_from_null() {
        let owned = CString::new("").unwrap();
        // SAFETY: valid NUL-terminated empty string.
        assert_eq!(unsafe { ptr_to_str(owned.as_ptr()) }, Some(""));
    }

    #[test]
    fn ref_round_trip() {
        let value = 42u32;
        // SAFETY: `&value` is a valid, aligned, initialised `u32`.
        assert_eq!(unsafe { ptr_to_ref(std::ptr::from_ref(&value)) }, Some(&42));
        // SAFETY: null is explicitly allowed by the contract.
        assert_eq!(unsafe { ptr_to_ref(std::ptr::null::<u32>()) }, None);
    }
}
