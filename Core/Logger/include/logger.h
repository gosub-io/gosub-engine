#ifndef LOGGER_H
#define LOGGER_H

#include <stdint.h>
#include <time.h>

// Logger initialization
void init_logger();

// Log memory usage in kilobytes
void log_memory_usage();

// Log response time in milliseconds
void log_response_time(clock_t start_time);

// Log parsing time in milliseconds
void log_parsing_time(clock_t start_time);

// Log DNS query speed in milliseconds
void log_dns_query_speed(clock_t start_time);

// Log blocking time in milliseconds
void log_blocking_time(clock_t start_time);

// Output collected information
void output_logs();

#endif // LOGGER_H
