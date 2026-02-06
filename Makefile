prefix?=/usr/local

lib:
	cargo build --release
	cp target/release/libbindle_file.a .
	cp target/release/libbindle_file.so libbindle.so || cp target/release/libbindle_file.dylib libbindle.dylib

install:
	cp include/bindle.h "$(prefix)/include/bindle.h"
	cp libbindle.* "$(prefix)/lib"

uninstall:
	rm -f "$(prefix)/include/bindle.h" "$(prefix)/lib/libbindle.*"

test:
	cargo test
