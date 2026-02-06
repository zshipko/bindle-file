# Bindle File Format (.bndl)

Bindle is a simple append-only binary archive format. It features a trailing index to support efficient writes and memory-mapped reads.

---

## 1. High-Level Layout

The file contains an 8 byte signature, followed by the data and then the metadata map at the end of the file.

| Offset | Component | Description |
| :--- | :--- | :--- |
| `0x00` | **Header** | 8-byte magic identification string. |
| `0x08` | **Data Payload** | Sequential blobs of raw or compressed data. |
| `Variable` | **Index** | A sequence of metadata entries and filenames. |
| `EOF - 16` | **Footer** | Pointer to the index and file count. |

---

## 2. Components

### 2.1 Header
Every Bindle file MUST begin with the following 8 bytes:
`42 49 4e 44 4c 30 30 31` (ASCII: `BINDL001`)

### 2.2 Data Segment
Data blobs are stored starting at offset `0x08`. 
- Each blob SHOULD be aligned to an **8-byte boundary** to ensure optimal performance when memory-mapping the file.
- Data can be stored as-is (Raw) or compressed using **Zstandard (zstd)**.

### 2.3 Index Entry (`Entry`)
The index consists of a series of entries. Each entry is a fixed-size header followed immediately by a variable-length UTF-8 filename.

| Field | Size | Type | Description |
| :--- | :--- | :--- | :--- |
| `offset` | 8 bytes | u64 | Absolute file offset to start of data. |
| `c_size` | 8 bytes | u64 | Compressed size on disk. |
| `u_size` | 8 bytes | u64 | Original uncompressed size. |
| `crc32` | 4 bytes | u32 | Checksum of the stored data. |
| `name_len` | 2 bytes | u16 | Length of the following filename string. |
| `comp_type` | 1 byte | u8 | `0` = Raw, `1` = Zstd. |
| `reserved` | 1 byte | u8 | Alignment padding. |
| `filename` | variable | utf8 | Filename string

**Padding:** After the filename string, the file MUST be padded with null bytes until the next 8-byte boundary is reached.

### 2.4 Footer
The last 16 bytes of the file contain the lookup information required to parse the archive.

| Field | Size | Type | Description |
| :--- | :--- | :--- | :--- |
| `index_offset` | 8 bytes | u64 | Absolute offset to the start of the Index. |
| `entry_count` | 8 bytes | u32 | Total number of entries in the file. |

---

## 3. Implementation Guidelines

### 3.1 Reading Logic
To read a Bindle file:
1. Validate the file size (must be at least 28 bytes).
2. Read the first 8 bytes and the last 8 bytes to verify the `BINDL001` magic.
3. Read the `index_offset` from the footer (EOF - 16).
4. Seek to `index_offset` and iterate `entry_count` times to populate an in-memory map of files.

### 3.2 Writing Logic (Atomic Updates)
To maintain an append-only structure:
1. Seek to the `index_offset` found in the current footer (effectively overwriting the old index).
2. Append new data blobs.
3. Write a new Index containing all previous entries plus the new ones.
4. Write a new Footer.
5. Flush/Sync the file to disk.

### 3.3 Constraints
- **Unique Keys:** Duplicate filenames are not permitted.
- **Null Bytes:** Filenames MUST NOT contain internal null bytes (`\0`).
- **Maximum Size:** File offsets are 64-bit, supporting archives up to 16 Exabytes.

---

## 4. Design Rationale
- **Trailing Index:** Allows files to be "updated" or added by simply appending to the end of the file and writing a new index.
- **Alignment:** 8-byte alignment ensures that `u64` fields can be read directly from a memory-mapped pointer without unaligned access penalties on modern CPUs.
- **Zero-Copy:** Raw entries can be used directly as slices from memory without decompression or copying.
