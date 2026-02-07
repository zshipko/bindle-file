# bindle-file

[bindle](https://en.wikipedia.org/wiki/Bindle) is an efficient, general purpose binary archive format for collecting files.

The format uses memory-mapped I/O for fast reads, optional zstd compression, and supports append-only writes with shadowing for updates. Files can be added incrementally without rewriting the entire archive.

## Usage

```rust
use bindle_file::{Bindle, Compress};

// Create or open an archive
let mut archive = Bindle::open("data.bndl")?;

// Add files
archive.add("config.json", data, Compress::None)?;
archive.save()?;

// Read files
let data = archive.read("config.json").unwrap();

// Update by shadowing (old data remains until vacuum)
archive.add("config.json", new_data, Compress::None)?;
archive.save()?;

// Reclaim space from shadowed entries
archive.vacuum()?;
```

## C API

The library includes C bindings:

```c
#include "bindle.h"

Bindle* bindle = bindle_open("data.bndl");
bindle_add(bindle, "file.txt", data, len, BindleCompressNone);
bindle_save(bindle);

size_t size;
uint8_t* data = bindle_read(bindle, "file.txt", &size);

// Or for uncompressed entries, read directly without decompression
uint8_t* raw = bindle_read_uncompressed_direct(bindle, "file.txt", &size);

free(data);
bindle_close(bindle);
```

Run:

```sh
make build
```

To build `libbindle` and copy in to the root of repository

## CLI

The `bindle` command provides basic operations:

```bash
bindle add archive.bndl file.txt
bindle read archive.bndl file.txt
bindle list archive.bndl
bindle vacuum archive.bndl
```

## Format

See [SPEC.md](SPEC.md) for the binary format specification.
