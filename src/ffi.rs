use std::alloc::{Layout, dealloc};
use std::ffi::CStr;
use std::io::{Read, Write};
use std::mem;
use std::os::raw::c_char;
use std::slice;

use crate::{Bindle, Compress, Reader, Writer};

/// Open a bindle file from disk, the path paramter should be NUL terminated
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

/// Adds a new entry, the name should be NUL terminated, will the data can contain NUL characters since the length
/// is provided
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_add(
    ctx: *mut Bindle,
    name: *const c_char,
    data: *const u8,
    data_len: usize,
    compress: Compress,
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

/// Adds a new entry, the name should be NUL terminated, will the data can contain NUL characters since the length
/// is provided
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_add_file(
    ctx: *mut Bindle,
    name: *const c_char,
    path: *const c_char,
    compress: Compress,
) -> bool {
    if ctx.is_null() || name.is_null() || path.is_null() {
        return false;
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let path_str = match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let b = &mut (*ctx);

        b.add_file(name_str, path_str, compress).is_ok()
    }
}

/// Save any changed to disk
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

/// Close an open bindle file
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_close(ctx: *mut Bindle) {
    if ctx.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(ctx)) }
}

/// Read a value from a bindle file in memory, returns a pointer that should be freed with
/// `bindle_free_buffer`
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

/// Used to free the results from `bindle_read`
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

/// Directly read an uncompressed entry from disk, returns NULL if the entry is compressed or doesn't exist
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

/// Get the number of entries in a bindle file
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

/// Compact and rewrite bindle file
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_vacuum(ctx: *mut Bindle) -> bool {
    if ctx.is_null() {
        return false;
    }
    let b = unsafe { &mut (*ctx) };
    b.vacuum().is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_unpack(ctx: *mut Bindle, dest_path: *const c_char) -> bool {
    if ctx.is_null() || dest_path.is_null() {
        return false;
    }
    let b = unsafe { &*ctx };
    let path = unsafe { CStr::from_ptr(dest_path).to_string_lossy() };
    b.unpack(path.as_ref()).is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_pack(
    ctx: *mut Bindle,
    src_path: *const c_char,
    compress: Compress,
) -> bool {
    if ctx.is_null() || src_path.is_null() {
        return false;
    }
    let b = unsafe { &mut *ctx };
    let path = unsafe { CStr::from_ptr(src_path).to_string_lossy() };
    b.pack(path.as_ref(), compress).is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_exists(ctx: *const Bindle, name: *const c_char) -> bool {
    if ctx.is_null() || name.is_null() {
        return false;
    }

    let b = unsafe { &*ctx };
    let name_str = unsafe {
        match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        }
    };

    b.exists(name_str)
}

/// Create a new Writer, while the stream is active (until bindle_stream_finish is called), the
/// Bindle struct should not be accessed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_writer_new<'a>(
    ctx: *mut Bindle,
    name: *const c_char,
    compress: Compress,
) -> *mut Writer<'a> {
    unsafe {
        let b = &mut *ctx;
        let name_str = CStr::from_ptr(name).to_string_lossy();

        match b.writer(&name_str, compress) {
            Ok(stream) => Box::into_raw(Box::new(std::mem::transmute(stream))),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_writer_write(
    stream: *mut Writer,
    data: *const u8,
    len: usize,
) -> bool {
    unsafe {
        let s = &mut *stream;
        let chunk = std::slice::from_raw_parts(data, len);
        s.write_all(chunk).is_ok()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_writer_close(stream: *mut Writer) -> bool {
    let s = unsafe { Box::from_raw(stream) };
    s.close().is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_reader_new<'a>(
    ctx: *const Bindle,
    name: *const c_char,
) -> *mut Reader<'a> {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }

    let b = unsafe { &*ctx };
    let name_str = unsafe { CStr::from_ptr(name).to_string_lossy() };

    match b.reader(&name_str) {
        Ok(reader) => Box::into_raw(Box::new(reader)),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_reader_read(
    reader: *mut Reader,
    buffer: *mut u8,
    buffer_len: usize,
) -> isize {
    if reader.is_null() || buffer.is_null() {
        return -1;
    }

    let r = unsafe { &mut *reader };
    let out_slice = unsafe { slice::from_raw_parts_mut(buffer, buffer_len) };

    match r.read(out_slice) {
        Ok(n) => n as isize,
        Err(_) => -1,
    }
}

/// Verify the CRC32 of data read from the reader.
/// Should be called after reading all data to ensure integrity.
/// Returns true if CRC32 matches, false otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_reader_verify_crc32(reader: *const Reader) -> bool {
    if reader.is_null() {
        return false;
    }

    let r = unsafe { &*reader };
    r.verify_crc32().is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_reader_close(reader: *mut Reader) {
    if !reader.is_null() {
        unsafe {
            drop(Box::from_raw(reader));
        }
    }
}
