#ifndef ERROR_HANDLER_H
#define ERROR_HANDLER_H

#include <stdio.h>
#include <stdlib.h>

// error codes
typedef enum {
    ERROR_NONE = 0,
    ERROR_INVALID_INPUT = 1,
    ERROR_FILE_NOT_FOUND = 2,
    NOT_FOUND = 404,
    BAD_REQUEST = 400,
    UNAUTHORIZED = 401,
    REQUEST_TIMEOUT = 408,
    INTERNAL_SERVER_ERROR = 500,
    BAD_GATEWAY = 502,
    SERVICE_UNAVALIBLE = 503,
    GATEWAY_TIMEOUT = 504,
} ErrorCode;

// Function to handle errors
void handle_error(ErrorCode code, const char* message);

#endif // ERROR_HANDLER_H

//simple error handler header file, needs more working on...
