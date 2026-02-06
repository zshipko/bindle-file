lib:
	cargo build --release
	cp target/release/libbindle_file.a .
	cp target/release/libbindle_file.so libbindle.so || cp target/release/libbindle_file.dylib libbindle.dylib

install:
	cp include/bindle.h /usr/local/include/bindle.h
	cp libbbindle.* /usr/local/lib

uninstall:
	rm -f /usr/local/include/bindle.h /usr/local/lib/libbindle.*

test:
	cargo test
