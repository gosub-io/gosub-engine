#ifndef CLIENT_STORAGE_H
#define CLIENT_STORAGE_H

#include <stdbool.h>
#include <stdint.h>

// Define maximum length for profile name
#define MAX_PROFILE_NAME_LENGTH 256

// Define maximum length for data key
#define MAX_DATA_KEY_LENGTH 256

// Profile structure to store client-side data
typedef struct {
    char name[MAX_PROFILE_NAME_LENGTH];
    // Add more fields as needed
} Profile;

// Initialize client storage
void init_client_storage();

// Create a new profile
Profile* create_profile(const char* profile_name);

// Store data in the specified profile
void store_data(Profile* profile, const char* key, const char* value);

// Retrieve data from the specified profile
const char* get_data(Profile* profile, const char* key);

// Clear data from the specified profile
void clear_data(Profile* profile, const char* key);

// Clear all data from the specified profile
void clear_all_data(Profile* profile);

// Set session-based persistence for local storage
void set_session_persistence(bool enable);

// Configure cookies based on security standards
void configure_cookies(bool block_third_party);

#endif // CLIENT_STORAGE_H
