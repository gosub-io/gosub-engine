#include <stdio.h>

#if defined(_WIN32)
#include <Windows.h>
#elif defined(__linux__)
#include <sys/utsname.h>
#elif defined(__APPLE__)
#include <sys/sysctl.h>
#endif

void getOSInformation() {
#if defined(_WIN32)
    // Windows-specific code
    OSVERSIONINFO osVersion;
    osVersion.dwOSVersionInfoSize = sizeof(OSVERSIONINFO);
    GetVersionEx(&osVersion);

    printf("Windows %d.%d (Build %d)\n", osVersion.dwMajorVersion, osVersion.dwMinorVersion, osVersion.dwBuildNumber);
#elif defined(__linux__)
    // Linux-specific code
    struct utsname unameData;
    uname(&unameData);

    printf("%s %s %s\n", unameData.sysname, unameData.release, unameData.machine);
#elif defined(__APPLE__)
    // macOS-specific code
    size_t len;
    char version[256];
    sysctlbyname("kern.osrelease", version, &len, NULL, 0);

    printf("macOS %s\n", version);
#else
    printf("Unsupported Operating System\n");
#endif
}

int main() {
    getOSInformation();
    return 0;
}
// this code is basic and it does not cover all the parts
