#ifndef CONFIG_STORE_H
#define CONFIG_STORE_H

#include <stdbool.h>

// Maximum length for configuration key and value
#define MAX_CONFIG_KEY_LENGTH 256
#define MAX_CONFIG_VALUE_LENGTH 512

// Structure to represent a configuration setting
typedef struct {
    char key[MAX_CONFIG_KEY_LENGTH];
    char value[MAX_CONFIG_VALUE_LENGTH];
} ConfigSetting;

// Configuration store interface
typedef struct {
    // Function to get a configuration setting by key
    const char* (*get_config)(const char* key);

    // Function to set a configuration setting
    void (*set_config)(const char* key, const char* value);
} ConfigStore;

// Function to create and initialize a configuration store
ConfigStore* create_config_store();

#endif // CONFIG_STORE_H
