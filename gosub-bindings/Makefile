# "global" compile settings
INCLUDE_DIR := include
SRC_DIR := src
CPPFLAGS := -I$(INCLUDE_DIR) -I$(INCLUDE_DIR)/nodes
CFLAGS_DEBUG := -std=c99 -g -Wall -Wextra -O0
CFLAGS_RELEASE := -std=c99 -Wall -Wextra -O2
CFLAGS := $(CFLAGS_DEBUG)
CC := gcc

LDFLAGS := -L./target/debug
NODE_SRC_DIR := $(SRC_DIR)/nodes
bindings: # build gosub static library to be used externally
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $(SRC_DIR)/gosub_api.c $(SRC_DIR)/nodes.c $(NODE_SRC_DIR)/text.c
	ar rcs libgosub.a gosub_api.o nodes.o text.o
	rm *.o
	test -d lib || mkdir lib
	cp ./target/debug/libgosub_bindings.a lib/
	cp ./libgosub.a lib/

TEST_SRC_DIR := tests
test: # build and run tests for bindings
	$(CC) $(TEST_SRC_DIR)/render_tree_test.c -L./ -lgosub -L./target/debug -lgosub_bindings -lsqlite3 -lm -o $(TEST_SRC_DIR)/render_tree_test $(CPPFLAGS) $(CFLAGS)
	./$(TEST_SRC_DIR)/render_tree_test

format:
	cargo clippy --fix --allow-dirty --allow-staged
