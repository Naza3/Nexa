/* Nexa installer code uses only Windows inbox libraries, not a CRT. */
#ifndef NEXA_INSTALLER_NATIVE_H
#define NEXA_INSTALLER_NATIVE_H
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shellapi.h>
#include <shlobj.h>
#include <knownfolders.h>

static void zero_bytes(void *address, SIZE_T count) {
    volatile unsigned char *p = (volatile unsigned char *)address;
    while (count--) *p++ = 0;
}
static BOOL append_text(WCHAR *destination, DWORD capacity, const WCHAR *source) {
    DWORD used = (DWORD)lstrlenW(destination), extra = (DWORD)lstrlenW(source);
    if (used >= capacity || extra >= capacity - used) return FALSE;
    lstrcpyW(destination + used, source);
    return TRUE;
}
/* No filesystem write: missing tails are permitted, links and access failures
   are not. Check ancestors too, including redirected profile paths. */
static BOOL safe_path(const WCHAR *path) {
    WCHAR *part;
    DWORD length = (DWORD)lstrlenW(path), i;
    BOOL result = TRUE;
    if (length < 3 || length >= 32768 || path[1] != L':' || path[2] != L'\\') return FALSE;
    part = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, (length + 1) * sizeof(WCHAR));
    if (!part) return FALSE;
    lstrcpyW(part, path);
    for (i = 3; i <= length; ++i) {
        if (part[i] == L'\\' || part[i] == 0) {
            WCHAR saved = part[i];
            DWORD attributes, error;
            part[i] = 0;
            attributes = GetFileAttributesW(part);
            error = GetLastError();
            part[i] = saved;
            if (attributes == INVALID_FILE_ATTRIBUTES) {
                if (error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND) { result = FALSE; break; }
            } else if (attributes & FILE_ATTRIBUTE_REPARSE_POINT) { result = FALSE; break; }
        }
    }
    HeapFree(GetProcessHeap(), 0, part);
    return result;
}
#endif
