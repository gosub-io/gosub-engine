#include "error_handler.h"

// Function to handle errors
void handle_error(ErrorCode code, const char* message) {
    fprintf(stderr, "Error Code: %d\n", code);
    fprintf(stderr, "Error Message: %s\n", message);

    // More logic will be implented here
    exit(code);
}
