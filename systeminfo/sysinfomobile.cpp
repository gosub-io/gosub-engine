#include <iostream>

#if defined(__ANDROID__)
#include <sys/system_properties.h>
#elif defined(__APPLE__) && defined(__MACH__)
#include <sys/utsname.h>
#endif

void getOSInformation() {
#if defined(__ANDROID__)
    // Android-specific code
    char osVersion[PROP_VALUE_MAX];
    __system_property_get("ro.build.version.release", osVersion);

    std::cout << "Android " << osVersion << "\n";
#elif defined(__APPLE__) && defined(__MACH__)
    // iOS-specific code
    struct utsname unameData;
    uname(&unameData);

    std::cout << "iOS " << unameData.release << "\n";
#else
    std::cout << "Unsupported Operating System\n";
#endif
}

int main() {
    getOSInformation();
    return 0;
}

// this code is basic and it does not cover all the parts for a functional system information fetcher
