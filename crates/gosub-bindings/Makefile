# "global" compile settings

# compilation mode Release or Debug
MODE?=Release

CFLAGS_DEBUG := -std=c99 -g -Wall -Wextra -O0
CFLAGS_RELEASE := -std=c99 -Wall -Wextra -O2

TARGET_DIR_DEBUG := ../../target/debug
TARGET_DIR_RELEASE := ../../target/release

ifeq ($(MODE), Release)
$(info ***** COMPILING IN RELEASE MODE *****)
CFLAGS = $(CFLAGS_RELEASE)
TARGET_DIR = $(TARGET_DIR_RELEASE)
else ifeq ($(MODE), Debug)
$(info ***** COMPILING IN DEBUG MODE *****)
CFLAGS = $(CFLAGS_DEBUG)
TARGET_DIR = $(TARGET_DIR_DEBUG)
else
$(warning ***** UNKNOWN MODE. DEFAULTING TO RELEASE MODE *****)
CFLAGS = $(CFLAGS_RELEASE)
TARGET_DIR = $(TARGET_DIR_RELEASE)
endif

CC := gcc

INCLUDE_DIR := include
SRC_DIR := src
NODE_SRC_DIR := $(SRC_DIR)/nodes

CPPFLAGS := -I$(INCLUDE_DIR) -I$(INCLUDE_DIR)/nodes
LDFLAGS := -L$(TARGET_DIR)

bindings: # build gosub static library to be used externally
ifeq ($(MODE), Debug)
	cargo build
else
	cargo build --release
endif
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $(SRC_DIR)/gosub_api.c $(SRC_DIR)/nodes.c $(NODE_SRC_DIR)/text.c
	ar rcs libgosub.a gosub_api.o nodes.o text.o
	rm *.o
	test -d lib || mkdir lib
	cp $(TARGET_DIR)/libgosub_bindings.a lib/
	cp ./libgosub.a lib/

TEST_SRC_DIR := tests
test: # build and run tests for bindings
	$(CC) $(TEST_SRC_DIR)/rendertree_test.c -L./ -lgosub $(LDFLAGS) -lgosub_bindings -lsqlite3 -lm -o $(TEST_SRC_DIR)/rendertree_test $(CPPFLAGS) $(CFLAGS)
	./$(TEST_SRC_DIR)/rendertree_test

format:
	cargo clippy --fix --allow-dirty --allow-staged
