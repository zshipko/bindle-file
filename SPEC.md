# Bindle File Format

Bindle is an append-only binary archive format designed for efficient writes, logical updates via shadowing, and memory-mapped reads.

---

## 1. High-Level Layout

A Bindle file consists of a fixed header, a data payload area, a trailing index, and a fixed-size footer.

| Offset | Component | Description |
| :--- | :--- | :--- |
| `0x00` | **Header** | 8-byte magic identification string |
| `0x08` | **Data Payload** | Sequential blobs of raw or compressed data |
| `Variable` | **Index** | Sequence of `Entry` headers and filenames |
| `EOF - 16` | **Footer** | 16-byte tail containing the Index pointer and count |

---

## 2. Component Details

### 2.1 Header
Every Bindle file MUST begin with exactly 8 bytes:
`42 49 4e 44 4c 30 30 31` (ASCII: `BINDL001`).

### 2.2 Data Segment
Data blobs begin at offset `0x08`.
- **Alignment:** Every data blob MUST be padded with null bytes to an **8-byte boundary**.
- **Compression:** Blobs may be raw or compressed via Zstd.
- **Shadowing:** New versions of existing files are simply appended to the end of the data segment. The file remains append-only until a vacuum operation is performed.

### 2.3 Index Entry
The index is a series of entries. Each entry consists of a fixed metadata block followed by a variable-length filename.

| Field | Size | Type | Description |
| :--- | :--- | :--- | :--- |
| `offset` | 8 bytes | u64 | Absolute file offset to the data blob |
| `c_size` | 8 bytes | u64 | Compressed size on disk |
| `u_size` | 8 bytes | u64 | Original uncompressed size |
| `crc32` | 4 bytes | u32 | Checksum of the stored data |
| `name_len` | 2 bytes | u16 | Length of the filename string |
| `comp_type` | 1 byte | u8 | `0` = Raw, `1` = Zstandard |
| `reserved` | 1 byte | u8 | Alignment padding |
| `filename` | Variable | UTF-8 | The entry name |

**Padding:** After the filename, the file MUST be padded with null bytes (`\0`) to the next 8-byte boundary before the next entry begins.

### 2.4 Footer
The last 16 bytes of the file are used to locate the index. Both fields are stored in little-endian format.

| Field | Size | Type | Description |
| :--- | :--- | :--- | :--- |
| `index_offset` | 8 bytes | u64 | Absolute offset to the start of the index |
| `entry_count` | 4 bytes | u32 | Total number of unique entries in the index |
| `magic`       | 4 bytes | u32 | Magic sentinel number

---

## 3. Operational Logic

### 3.1 Shadowing & Atomic Updates
To "update" a file or add new ones:
1. Append new data starting at the current `index_offset`.
2. Write a new Index. If a filename is repeated, the index points to the **newest** data offset.
3. Write a new Footer.
4. Old data remains in the file (unreferenced) until a vacuum occurs.

### 3.2 Vacuuming
To reclaim space used by shadowed data:
1. Create a temporary file and write the `BINDL001` header.
2. Iterate through the **live** index entries only.
3. Copy the referenced data blobs to the new file, updating their offsets in a new in-memory index.
4. Write the new Index and Footer to the temporary file.
5. Atomically replace the old file with the new one.

---

## 4. Design Rationale
- **Trailing Index:** Enables "single-pass" appending. You don't need to shift existing data to grow the index.
- **8-Byte Alignment:** Ensures that all 64-bit integers in the metadata and footer are naturally aligned, preventing performance penalties on architectures that dislike unaligned reads.
- **Zero-Copy Potential:** Raw (uncompressed) data blobs can be used directly as memory slices via `mmap` without intermediate buffers.
