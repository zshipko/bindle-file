.PHONY: test
test: check
	python3 test.py a.bindl

	
check:
	clang -I .. -o bindle_c bindle-cli.c bindle.c `pkg-config --cflags --libs libzstd`
	
