use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;

use crate::Bindle;

/// Opaque handle to a Bindle archive.
pub struct BindleContext {
    pub(crate) inner: Bindle,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_open(path: *const c_char) -> *mut BindleContext {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    // Explicit unsafe block for raw pointer dereference
    let path_str = unsafe {
        match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    match Bindle::open(path_str) {
        Ok(b) => Box::into_raw(Box::new(BindleContext { inner: b })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Adds a new entry. Returns true on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_add(
    ctx: *mut BindleContext,
    name: *const c_char,
    data: *const u8,
    data_len: usize,
    compress: bool,
) -> bool {
    if ctx.is_null() || name.is_null() || (data.is_null() && data_len > 0) {
        return false;
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let data_slice = slice::from_raw_parts(data, data_len);
        let b = &mut (*ctx).inner;

        b.add(name_str, data_slice, compress).is_ok()
    }
}

/// Commits changes to disk.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_save(ctx: *mut BindleContext) -> bool {
    if ctx.is_null() {
        return false;
    }
    unsafe {
        let b = &mut (*ctx).inner;
        b.save().is_ok()
    }
}

/// Frees BindleContext
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_free(ctx: *mut BindleContext) {
    if ctx.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(ctx)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_read(
    ctx: *mut BindleContext,
    name: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    if ctx.is_null() || name.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let b = &(*ctx).inner;

        if let Some(data) = b.read(name_str) {
            let mut bytes = data.to_vec();
            bytes.shrink_to_fit();
            let ptr = bytes.as_mut_ptr();
            *out_len = bytes.len();
            std::mem::forget(bytes);
            ptr
        } else {
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_free_buffer(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe {
            let _ = Vec::from_raw_parts(ptr, len, len);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_length(ctx: *const BindleContext) -> usize {
    if ctx.is_null() {
        return 0;
    }
    unsafe { (*ctx).inner.len() }
}

/// Returns the name of the entry at the given index.
/// The string is owned by the Bindle; the caller must NOT free it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_entry_name(
    ctx: *const BindleContext,
    index: usize,
    len: *mut usize,
) -> *const c_char {
    if ctx.is_null() {
        return std::ptr::null();
    }

    let b = unsafe { &(*ctx).inner };
    match b.entries.get(index) {
        Some((_, name)) => {
            unsafe {
                *len = name.as_bytes().len();
            }
            name.as_ptr() as *const _
        }
        None => std::ptr::null(),
    }
}
