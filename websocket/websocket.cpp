
#include <iostream>
#include <cstring>
#include <arpa/inet.h> // for htonl

#include "websocket.h"

// Function to parse a WebSocket header
int parse_websocket_header(const char* data, size_t len, struct WebSocketHeader* header) {
    if (len < 2) {
        // Incomplete header
        return -1;
    }

    // Parse the first byte
    header->opcode = data[0] & 0x0F;
    header->is_masked = (data[1] & 0x80) >> 7;

    // Parse the payload length
    uint8_t len_byte = data[1] & 0x7F;
    if (len_byte < 126) {
        header->payload_length = len_byte;
    } else if (len_byte == 126) {
        if (len < 4) {
            // Incomplete header
            return -1;
        }
        header->payload_length = ntohs(*reinterpret_cast<const uint16_t*>(data + 2));
    } else {
        if (len < 10) {
            // Incomplete header
            return -1;
        }
        header->payload_length = ntohll(*reinterpret_cast<const uint64_t*>(data + 2));
    }

    // Parse masking key if present
    if (header->is_masked && len >= 4 + (size_t)header->payload_length) {
        std::memcpy(header->masking_key, data + (header->payload_length == 126 ? 4 : 10), 4);
    }

    return 0;
}

// Function to build a WebSocket header
int build_websocket_header(enum WebSocketOpcode opcode, const char* payload, size_t payload_len, char* buffer, size_t buffer_size) {
    if (buffer_size < 2) {
        // Insufficient buffer size
        return -1;
    }

    // Set opcode
    buffer[0] = opcode;

    // Set payload length and masking flag
    uint8_t* payload_length_field = nullptr;
    if (payload_len < 126) {
        buffer[1] = static_cast<uint8_t>(payload_len);
    } else if (payload_len <= 0xFFFF) {
        buffer[1] = 126;
        payload_length_field = reinterpret_cast<uint8_t*>(&buffer[2]);
        *reinterpret_cast<uint16_t*>(payload_length_field) = htons(static_cast<uint16_t>(payload_len));
    } else {
        buffer[1] = 127;
        payload_length_field = reinterpret_cast<uint8_t*>(&buffer[2]);
        *reinterpret_cast<uint64_t*>(payload_length_field) = htonll(static_cast<uint64_t>(payload_len));
    }

    // Set masking key
    if (payload_len > 0) {
        buffer[1] |= 0x80; // Set the mask bit
        uint8_t* masking_key = reinterpret_cast<uint8_t*>(&buffer[buffer_size - 4]);
        for (size_t i = 0; i < 4; ++i) {
            masking_key[i] = static_cast<uint8_t>(rand() % 256);
        }
    }

    return 0;
}

int main() {
    // Example usage of the WebSocket header and functions
    const char* message = "Hello, WebSocket!";
    size_t message_len = std::strlen(message);

    // Build WebSocket frame
    const size_t buffer_size = 16;
    char buffer[buffer_size];
    build_websocket_header(WS_OPCODE_TEXT, message, message_len, buffer, buffer_size);

    // Parse WebSocket frame
    struct WebSocketHeader header;
    parse_websocket_header(buffer, buffer_size, &header);

    // Output parsed header information
    std::cout << "Parsed WebSocket Header:\n"
              << "Opcode: " << static_cast<int>(header.opcode) << "\n"
              << "Is Masked: " << header.is_masked << "\n"
              << "Payload Length: " << header.payload_length << "\n";

    return 0;
}
// this code is an basic example of an websocket , it needs more working
