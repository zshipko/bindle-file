.PHONY: test
test: check
	python3 test.py a.bindl
	clang -Wall -Wextra -Wpedantic -I ../include -o bindle_c bindle-cli.c bindle.c `pkg-config --cflags --libs libzstd`
	
check:
	clang -fsanitize=address -Wall -Wextra -Wpedantic -I ../include -o bindle_c bindle-cli.c bindle.c `pkg-config --cflags --libs libzstd`
	
