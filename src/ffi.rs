use std::alloc::{Layout, dealloc};
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::mem;
use std::os::raw::c_char;
use std::slice;

use crate::{Compress, Reader, Writer};

/// FFI wrapper around Bindle that caches null-terminated entry names for C API.
pub struct Bindle {
    bindle: crate::Bindle,
    entry_names_cache: Vec<CString>,
}

impl Bindle {
    fn new(bindle: crate::Bindle) -> Self {
        let mut ffi = Bindle {
            bindle,
            entry_names_cache: Vec::new(),
        };
        ffi.rebuild_cache();
        ffi
    }

    fn rebuild_cache(&mut self) {
        self.entry_names_cache.clear();
        for (name, _) in &self.bindle.index {
            if let Ok(c_str) = CString::new(name.as_str()) {
                self.entry_names_cache.push(c_str);
            }
        }
    }
}

/// Creates a new archive, overwriting any existing file.
///
/// # Parameters
/// * `path` - NUL-terminated path to the archive file
///
/// # Returns
/// A pointer to the Bindle handle, or NULL on error. Must be freed with `bindle_close()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_create(path: *const c_char) -> *mut Bindle {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    let path_str = unsafe {
        match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    match crate::Bindle::create(path_str) {
        Ok(b) => Box::into_raw(Box::new(Bindle::new(b))),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Opens an existing archive or creates a new one.
///
/// # Parameters
/// * `path` - NUL-terminated path to the archive file
///
/// # Returns
/// A pointer to the Bindle handle, or NULL on error. Must be freed with `bindle_close()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_open(path: *const c_char) -> *mut Bindle {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    let path_str = unsafe {
        match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    match crate::Bindle::open(path_str) {
        Ok(b) => Box::into_raw(Box::new(Bindle::new(b))),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Opens an existing archive. Returns NULL if the file doesn't exist.
///
/// # Parameters
/// * `path` - NUL-terminated path to the archive file
///
/// # Returns
/// A pointer to the Bindle handle, or NULL on error. Must be freed with `bindle_close()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_load(path: *const c_char) -> *mut Bindle {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    let path_str = unsafe {
        match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    match crate::Bindle::load(path_str) {
        Ok(b) => Box::into_raw(Box::new(Bindle::new(b))),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Adds data to the archive with the given name.
///
/// # Parameters
/// * `ctx` - Bindle handle from `bindle_open()`
/// * `name` - NUL-terminated entry name
/// * `data` - Data bytes (may contain NUL bytes)
/// * `data_len` - Length of data in bytes
/// * `compress` - Compression mode (BindleCompressNone, BindleCompressZstd, or BindleCompressAuto)
///
/// # Returns
/// True on success. Call `bindle_save()` to commit changes.
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

        let result = b.bindle.add(name_str, data_slice, compress).is_ok();
        if result {
            b.rebuild_cache();
        }
        result
    }
}

/// Adds a file from the filesystem to the archive.
///
/// # Parameters
/// * `ctx` - Bindle handle from `bindle_open()`
/// * `name` - NUL-terminated entry name
/// * `path` - NUL-terminated path to file on disk
/// * `compress` - Compression mode
///
/// # Returns
/// True on success. Call `bindle_save()` to commit changes.
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

        let result = b.bindle.add_file(name_str, path_str, compress).is_ok();
        if result {
            b.rebuild_cache();
        }
        result
    }
}

/// Commits all pending changes to disk.
///
/// Writes the index and footer. Must be called after add/remove operations.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_save(ctx: *mut Bindle) -> bool {
    if ctx.is_null() {
        return false;
    }
    unsafe {
        let b = &mut (*ctx);
        b.bindle.save().is_ok()
    }
}

/// Closes the archive and frees the handle.
///
/// After calling this, the ctx pointer is no longer valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_close(ctx: *mut Bindle) {
    if ctx.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(ctx)) }
}

/// Reads an entry from the archive, decompressing if needed.
///
/// # Parameters
/// * `ctx_ptr` - Bindle handle
/// * `name` - NUL-terminated entry name
/// * `out_len` - Output parameter for data length
///
/// # Returns
/// Pointer to data buffer, or NULL if not found or CRC32 check fails.
/// Must be freed with `bindle_free_buffer()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_read_buffer(
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
        match ctx.bindle.read(name_str) {
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

/// Frees a buffer returned by `bindle_read()`.
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

/// Reads an uncompressed entry without allocating.
///
/// Returns a pointer directly into the memory-mapped archive. Only works for uncompressed entries.
///
/// # Parameters
/// * `ctx` - Bindle handle
/// * `name` - NUL-terminated entry name
/// * `out_len` - Output parameter for data length
///
/// # Returns
/// Pointer into the mmap, or NULL if entry is compressed or doesn't exist.
/// The pointer is valid as long as the Bindle handle is open. Do NOT free this pointer.
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
        if let Some(data) = b.bindle.read(name_str) {
            match data {
                std::borrow::Cow::Borrowed(bytes) => bytes.as_ptr(),
                _ => std::ptr::null_mut(),
            }
        } else {
            std::ptr::null_mut()
        }
    }
}

/// Returns the number of entries in the archive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_length(ctx: *const Bindle) -> usize {
    if ctx.is_null() {
        return 0;
    }
    unsafe { (*ctx).bindle.len() }
}

/// Returns the name of the entry at the given index as a null-terminated C string.
///
/// Use with `bindle_length()` to iterate over all entries. The pointer is valid as long as the Bindle handle is open.
/// Do NOT free the returned pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_entry_name(ctx: *const Bindle, index: usize) -> *const c_char {
    if ctx.is_null() {
        return std::ptr::null();
    }

    let b = unsafe { &(*ctx) };
    match b.entry_names_cache.get(index) {
        Some(c_str) => c_str.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Reclaims space by removing shadowed data.
///
/// Rebuilds the archive with only live entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_vacuum(ctx: *mut Bindle) -> bool {
    if ctx.is_null() {
        return false;
    }
    let b = unsafe { &mut (*ctx) };
    let result = b.bindle.vacuum().is_ok();
    if result {
        b.rebuild_cache();
    }
    result
}

/// Extracts all entries to a destination directory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_unpack(ctx: *mut Bindle, dest_path: *const c_char) -> bool {
    if ctx.is_null() || dest_path.is_null() {
        return false;
    }
    let b = unsafe { &*ctx };
    let path = unsafe { CStr::from_ptr(dest_path).to_string_lossy() };
    b.bindle.unpack(path.as_ref()).is_ok()
}

/// Recursively adds all files from a directory to the archive.
///
/// Call `bindle_save()` to commit changes.
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
    let result = b.bindle.pack(path.as_ref(), compress).is_ok();
    if result {
        b.rebuild_cache();
    }
    result
}

/// Returns true if an entry with the given name exists.
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

    b.bindle.exists(name_str)
}

/// Removes an entry from the index.
///
/// Returns true if the entry existed. Data remains in the file until `bindle_vacuum()` is called.
/// Call `bindle_save()` to commit changes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_remove(ctx: *mut Bindle, name: *const c_char) -> bool {
    if ctx.is_null() || name.is_null() {
        return false;
    }

    let b = unsafe { &mut *ctx };
    let name_str = unsafe {
        match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        }
    };

    let result = b.bindle.remove(name_str);
    if result {
        b.rebuild_cache();
    }
    result
}

/// Creates a streaming writer for adding an entry.
///
/// The writer must be closed with `bindle_writer_close()`, then call `bindle_save()` to commit.
/// Do not access the Bindle handle while the writer is active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_writer_new<'a>(
    ctx: *mut Bindle,
    name: *const c_char,
    compress: Compress,
) -> *mut Writer<'a> {
    unsafe {
        let b = &mut *ctx;
        let name_str = CStr::from_ptr(name).to_string_lossy();

        match b.bindle.writer(&name_str, compress) {
            Ok(stream) => Box::into_raw(Box::new(std::mem::transmute(stream))),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

/// Writes data to the writer.
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

/// Closes the writer and finalizes the entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_writer_close(stream: *mut Writer) -> bool {
    let s = unsafe { Box::from_raw(stream) };
    s.close().is_ok()
}

/// Creates a streaming reader for an entry.
///
/// Automatically decompresses if needed. Must be freed with `bindle_reader_close()`.
/// Call `bindle_reader_verify_crc32()` after reading to verify integrity.
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

    match b.bindle.reader(&name_str) {
        Ok(reader) => Box::into_raw(Box::new(reader)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Reads data from the reader into the provided buffer.
///
/// Returns the number of bytes read, or -1 on error. Returns 0 on EOF.
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

/// Closes the reader and frees the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_reader_close(reader: *mut Reader) {
    if !reader.is_null() {
        unsafe {
            drop(Box::from_raw(reader));
        }
    }
}

/// Gets the uncompressed size of an entry by name.
///
/// # Parameters
/// * `ctx` - Bindle handle
/// * `name` - NUL-terminated entry name
///
/// # Returns
/// The uncompressed size in bytes, or 0 if the entry doesn't exist.
/// Note: Returns 0 for both non-existent entries and zero-length entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_entry_size(ctx: *const Bindle, name: *const c_char) -> usize {
    if ctx.is_null() || name.is_null() {
        return 0;
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let b = &*ctx;
        match b.bindle.index.get(name_str) {
            Some(entry) => entry.uncompressed_size() as usize,
            None => 0,
        }
    }
}

/// Gets the compression type of an entry by name.
///
/// # Parameters
/// * `ctx` - Bindle handle
/// * `name` - NUL-terminated entry name
///
/// # Returns
/// The Compress value (0 = None, 1 = Zstd), or 0 if the entry doesn't exist.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_entry_compress(ctx: *const Bindle, name: *const c_char) -> Compress {
    if ctx.is_null() || name.is_null() {
        return Compress::None;
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return Compress::None,
        };

        let b = &*ctx;
        match b.bindle.index.get(name_str) {
            Some(entry) => {
                if entry.compression_type == 1 {
                    Compress::Zstd
                } else {
                    Compress::None
                }
            }
            None => Compress::None,
        }
    }
}

/// Reads an entry into a pre-existing buffer.
///
/// Decompresses if needed and verifies CRC32. Reads up to `buffer_len` bytes.
///
/// # Parameters
/// * `ctx` - Bindle handle
/// * `name` - NUL-terminated entry name
/// * `buffer` - Pre-allocated buffer to read into
/// * `buffer_len` - Maximum number of bytes to read
///
/// # Returns
/// The number of bytes actually read, or 0 if the entry doesn't exist or CRC32 check fails.
/// If the entry is larger than `buffer_len`, only `buffer_len` bytes are read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bindle_read(
    ctx: *const Bindle,
    name: *const c_char,
    buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    if ctx.is_null() || name.is_null() || buffer.is_null() {
        return 0;
    }

    unsafe {
        let name_str = match CStr::from_ptr(name).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let b = &*ctx;
        let buffer_slice = slice::from_raw_parts_mut(buffer, buffer_len);

        match b.bindle.read_into(name_str, buffer_slice) {
            Ok(bytes_read) => bytes_read,
            Err(_) => 0,
        }
    }
}
