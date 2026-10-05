/* Use from manifest-bearing EXEs only. MSI DLL version compatibility is not a
   reliable source for this predicate. No filesystem, registry or network writes. */
#ifndef NEXA_OS_VERSION_H
#define NEXA_OS_VERSION_H
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
static BOOL windows10_or_later(void) {
    OSVERSIONINFOEXW version;
    volatile BYTE *bytes = (volatile BYTE *)&version;
    SIZE_T remaining = sizeof(version);
    ULONGLONG mask = 0;
    while (remaining--) *bytes++ = 0;
    version.dwOSVersionInfoSize = sizeof(version);
    version.dwMajorVersion = 10;
    mask = VerSetConditionMask(mask, VER_MAJORVERSION, VER_GREATER_EQUAL);
    mask = VerSetConditionMask(mask, VER_MINORVERSION, VER_GREATER_EQUAL);
    mask = VerSetConditionMask(mask, VER_SERVICEPACKMAJOR, VER_GREATER_EQUAL);
    mask = VerSetConditionMask(mask, VER_SERVICEPACKMINOR, VER_GREATER_EQUAL);
    return VerifyVersionInfoW(&version, VER_MAJORVERSION | VER_MINORVERSION | VER_SERVICEPACKMAJOR | VER_SERVICEPACKMINOR, mask);
}
#endif
