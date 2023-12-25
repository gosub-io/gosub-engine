#ifndef WEBSOCKET_H
#define WEBSOCKET_H

#include <stdint.h>

// Define WebSocket frame opcodes
enum WebSocketOpcode {
    WS_OPCODE_CONTINUATION = 0x0,
    WS_OPCODE_TEXT = 0x1,
    WS_OPCODE_BINARY = 0x2,
    WS_OPCODE_CLOSE = 0x8,
    WS_OPCODE_PING = 0x9,
    WS_OPCODE_PONG = 0xA
};

// WebSocket header structure
struct WebSocketHeader {
    uint8_t opcode;
    uint8_t is_masked;
    uint64_t payload_length;
    uint8_t masking_key[4];
};

// Function to parse a WebSocket header
int parse_websocket_header(const char* data, size_t len, struct WebSocketHeader* header);

// Function to build a WebSocket header
int build_websocket_header(enum WebSocketOpcode opcode, const char* payload, size_t payload_len, char* buffer, size_t buffer_size);

#endif // WEBSOCKET_H

// this code is an basic example of an websocket , it needs more working
