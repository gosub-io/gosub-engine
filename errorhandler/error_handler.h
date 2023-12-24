#ifndef ERROR_HANDLER_H
#define ERROR_HANDLER_H

#include <stdio.h>
#include <stdlib.h>

// Custom error codes
typedef enum {
    ERROR_NONE = 0,
    ERROR_INVALID_INPUT = 1,
    ERROR_FILE_NOT_FOUND = 2,
    // Add more error codes as needed
} ErrorCode;

// Function to handle errors
void handle_error(ErrorCode code, const char* message);

#endif // ERROR_HANDLER_H
