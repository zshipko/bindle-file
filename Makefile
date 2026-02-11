prefix?=/usr/local

build:
	cargo build --release
	cp target/release/libbindle_file.a ./libbindle.a
	cp target/release/libbindle_file.so libbindle.so || cp target/release/libbindle_file.dylib libbindle.dylib

install:
	cp include/bindle.h "$(prefix)/include/bindle.h"
	cp target/release/libbindle_file.a ."$(prefix)/lib/libbindle.a"
	-cp target/release/libbindle_file.so ."$(prefix)/lib/libbindle.so"
	-cp target/release/libbindle_file.dylib ."$(prefix)/lib/libbindle.dylib"

uninstall:
	rm -f "$(prefix)/include/bindle.h" "$(prefix)/lib/libbindle_file.*"

.PHONY: test
test:
	cargo test
	cd test && $(MAKE) test
