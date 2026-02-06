use std::alloc::{Layout, dealloc};
use std::ffi::CStr;
use std::mem;
use std::os::raw::c_char;
use std::slice;

use crate::Bindle;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_open(path: *const c_char) -> *mut Bindle {
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
        Ok(b) => Box::into_raw(Box::new(b)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Adds a new entry. Returns true on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_add(
    ctx: *mut Bindle,
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
        let b = &mut (*ctx);

        b.add(name_str, data_slice, compress).is_ok()
    }
}

/// Commits changes to disk.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_save(ctx: *mut Bindle) -> bool {
    if ctx.is_null() {
        return false;
    }
    unsafe {
        let b = &mut (*ctx);
        b.save().is_ok()
    }
}

/// Frees BindleContext
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_close(ctx: *mut Bindle) {
    if ctx.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(ctx)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_read(
    ctx_ptr: *mut Bindle,
    name: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    unsafe {
        if ctx_ptr.is_null() || name.is_null() {
            return std::ptr::null_mut();
        }

        // 1. Convert the C string to a Rust &str
        let c_str = std::ffi::CStr::from_ptr(name);
        let name_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // 2. Access your Rust Bindle struct
        let ctx = &mut *ctx_ptr;

        // 3. The actual data retrieval logic
        // (Assuming your Rust Bindle has a method like .get(name))
        match ctx.read(name_str) {
            Some(bytes) => wrap_in_ffi_header(bytes.as_ref(), out_len),
            None => return std::ptr::null_mut(),
        }
    }
}

/// Internal helper to perform the "Hidden Header" allocation
unsafe fn wrap_in_ffi_header(data: &[u8], out_len: *mut usize) -> *mut u8 {
    unsafe {
        let len = data.len();
        if !out_len.is_null() {
            *out_len = len;
        }

        let size_of_header = std::mem::size_of::<usize>();
        let total_size = size_of_header + len;
        let layout =
            std::alloc::Layout::from_size_align(total_size, std::mem::align_of::<usize>()).unwrap();

        let raw_ptr = std::alloc::alloc(layout);
        if raw_ptr.is_null() {
            return std::ptr::null_mut();
        }

        // Store the length at the start
        *(raw_ptr as *mut usize) = len;

        // Copy data to the payload area
        let data_ptr = raw_ptr.add(size_of_header);
        std::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, len);

        data_ptr
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_free_buffer(ptr: *mut u8) {
    unsafe {
        if ptr.is_null() {
            return;
        }

        let size_of_header = mem::size_of::<usize>();

        // 1. Step back to find the start of the header
        let raw_ptr = ptr.sub(size_of_header);

        // 2. Read the length we stored there
        let len = *(raw_ptr as *const usize);

        // 3. Reconstruct the layout used during allocation
        let total_size = size_of_header + len;
        let layout = Layout::from_size_align(total_size, mem::align_of::<usize>()).unwrap();

        // 4. Deallocate the entire block
        dealloc(raw_ptr, layout);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_read_uncompressed_direct(
    ctx: *mut Bindle,
    name: *const c_char,
    out_len: *mut usize,
) -> *const u8 {
    if ctx.is_null() || name.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let b = &(*ctx);
        if let Some(data) = b.read(name_str) {
            match data {
                std::borrow::Cow::Borrowed(bytes) => bytes.as_ptr(),
                _ => std::ptr::null_mut(),
            }
        } else {
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_length(ctx: *const Bindle) -> usize {
    if ctx.is_null() {
        return 0;
    }
    unsafe { (*ctx).len() }
}

/// Returns the name of the entry at the given index.
/// The string is owned by the Bindle; the caller must NOT free it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_entry_name(
    ctx: *const Bindle,
    index: usize,
    len: *mut usize,
) -> *const c_char {
    if ctx.is_null() {
        return std::ptr::null();
    }

    let b = unsafe { &(*ctx) };
    match b.index.iter().nth(index) {
        Some((name, _)) => {
            unsafe {
                *len = name.as_bytes().len();
            }
            name.as_ptr() as *const _
        }
        None => std::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_vacuum(ctx: *mut Bindle) -> bool {
    if ctx.is_null() {
        return false;
    }
    let b = unsafe { &mut (*ctx) };
    b.vacuum().is_ok()
}
