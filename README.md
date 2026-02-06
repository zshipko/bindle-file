# bindle-file

`bindle` is a general purpose, multi-file archive format with support for zstd and direct reads for uncompressed values.

This repository contains `bindle-file` for Rust, which can also be used to build C libraries in
`target/release/libbindle_file.a` and `target/release/libbindle_file.so`. `c/bindle.c` can also
be used as a drop-in replacement for the Rust implementation in C projects. 

