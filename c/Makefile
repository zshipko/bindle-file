.PHONY: test
test:
	clang -I .. -o bindle_c bindle-cli.c bindle.c `pkg-config --cflags --libs libzstd`
	python3 test.py a.bindl
	
