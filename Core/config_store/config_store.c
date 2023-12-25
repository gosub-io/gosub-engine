#include "config_store.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Maximum number of configuration settings
#define MAX_CONFIG_SETTINGS 10

// Array to store configuration settings
static ConfigSetting config_settings[MAX_CONFIG_SETTINGS];

// Number of configuration settings
static size_t num_config_settings = 0;

// Function to get a configuration setting by key
const char* get_config(const char* key) {
    for (size_t i = 0; i < num_config_settings; ++i) {
        ConfigSetting* setting = &config_settings[i];
        if (strcmp(setting->key, key) == 0) {
            return setting->value;
        }
    }
    return NULL; // Configuration not found
}

// Function to set a configuration setting
void set_config(const char* key, const char* value) {
    // Search for existing configuration
    for (size_t i = 0; i < num_config_settings; ++i) {
        ConfigSetting* setting = &config_settings[i];
        if (strcmp(setting->key, key) == 0) {
            strncpy(setting->value, value, MAX_CONFIG_VALUE_LENGTH);
            return;
        }
    }

    // Add a new configuration setting
    if (num_config_settings < MAX_CONFIG_SETTINGS) {
        ConfigSetting* setting = &config_settings[num_config_settings++];
        strncpy(setting->key, key, MAX_CONFIG_KEY_LENGTH);
        strncpy(setting->value, value, MAX_CONFIG_VALUE_LENGTH);
    } else {
        fprintf(stderr, "Error: Maximum number of configuration settings reached.\n");
    }
}

// Function to create and initialize a configuration store
ConfigStore* create_config_store() {
    ConfigStore* store = (ConfigStore*)malloc(sizeof(ConfigStore));
    if (store != NULL) {
        store->get_config = get_config;
        store->set_config = set_config;
    } else {
        fprintf(stderr, "Error: Failed to create configuration store.\n");
    }
    return store;
}
