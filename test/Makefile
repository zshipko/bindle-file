# Makefile for C API tests

CARGO_TARGET_DIR ?= ../target
RUST_LIB = $(CARGO_TARGET_DIR)/debug/libbindle_file.a
TEST_BINARY = runtest

# Detect OS
UNAME_S := $(shell uname -s)

ifeq ($(UNAME_S),Darwin)
	LDFLAGS = -framework Security -lSystem -lresolv -lc -lm
else
	LDFLAGS = -lpthread -ldl -lm
endif

all: test

$(RUST_LIB):
	cd .. && cargo build

$(TEST_BINARY): test.c $(RUST_LIB)
	$(CC) -o $(TEST_BINARY) test.c $(RUST_LIB) $(LDFLAGS)

test: $(TEST_BINARY)
	./$(TEST_BINARY)

clean:
	rm -f $(TEST_BINARY) *.bndl

.PHONY: all test clean
