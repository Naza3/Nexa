/* Shared read-only Windows installer path/process checks. */
#ifndef NEXA_GUARD_PATHS_H
#define NEXA_GUARD_PATHS_H
#include "native.h"
#include <tlhelp32.h>

static BOOL known_process(const WCHAR *name) {
    DWORD i;
    BOOL tilde = FALSE;
    WCHAR prefix[5];
    for (i = 0; name[i]; ++i) if (name[i] == L'~') tilde = TRUE;
    lstrcpynW(prefix, name, 5);
    if (tilde && !lstrcmpiW(prefix, L"NEXA")) return TRUE;
    lstrcpynW(prefix, name, 4);
    if (tilde && !lstrcmpiW(prefix, L"AI-")) return TRUE;
    return !lstrcmpiW(name, L"nexa-desktop.exe") || !lstrcmpiW(name, L"ai-runtime.exe") ||
           !lstrcmpiW(name, L"ai-runtime-worker.exe") || !lstrcmpiW(name, L"nexa-aria2.exe");
}
static BOOL process_inside(const WCHAR *root, const WCHAR *path) {
    DWORD i, count = (DWORD)lstrlenW(root);
    if ((DWORD)lstrlenW(path) < count) return FALSE;
    for (i = 0; i < count; ++i) {
        WCHAR a[2], b[2];
        a[0] = root[i]; a[1] = 0; b[0] = path[i]; b[1] = 0;
        if (lstrcmpiW(a, b)) return FALSE;
    }
    return TRUE; /* root includes a trailing separator */
}
static BOOL final_path(const WCHAR *path, WCHAR *output) {
    HANDLE file = CreateFileW(path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                              NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    DWORD size;
    if (file == INVALID_HANDLE_VALUE) return FALSE;
    size = GetFinalPathNameByHandleW(file, output, 32768, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
    CloseHandle(file);
    return size > 0 && size < 32768;
}
static BOOL stopped(const WCHAR *root) {
    HANDLE snapshot;
    PROCESSENTRY32W entry;
    BOOL result = TRUE, more;
    WCHAR *buffer, *canonical_root, *image, *canonical_image;
    DWORD attributes = GetFileAttributesW(root), error = GetLastError();
    if (attributes == INVALID_FILE_ATTRIBUTES && (error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND)) return TRUE;
    buffer = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, 3 * 32768 * sizeof(WCHAR));
    if (!buffer) return FALSE;
    canonical_root = buffer; image = buffer + 32768; canonical_image = buffer + 65536;
    if (!final_path(root, canonical_root) || (canonical_root[lstrlenW(canonical_root)-1] != L'\\' && !append_text(canonical_root, 32768, L"\\"))) {
        HeapFree(GetProcessHeap(), 0, buffer); return FALSE;
    }
    snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snapshot == INVALID_HANDLE_VALUE) { HeapFree(GetProcessHeap(), 0, buffer); return FALSE; }
    zero_bytes(&entry, sizeof(entry)); entry.dwSize = sizeof(entry);
    more = Process32FirstW(snapshot, &entry);
    if (!more && GetLastError() != ERROR_NO_MORE_FILES) result = FALSE;
    while (more && result) {
        if (known_process(entry.szExeFile)) {
            HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, entry.th32ProcessID);
            DWORD size = 32768;
            if (!process) result = FALSE;
            else {
                if (!QueryFullProcessImageNameW(process, 0, image, &size) || !final_path(image, canonical_image) || process_inside(canonical_root, canonical_image)) result = FALSE;
                CloseHandle(process);
            }
        }
        if (result) {
            more = Process32NextW(snapshot, &entry);
            if (!more && GetLastError() != ERROR_NO_MORE_FILES) result = FALSE;
        }
    }
    CloseHandle(snapshot);
    HeapFree(GetProcessHeap(), 0, buffer);
    return result;
}
/* Expand only existing ancestors, preserving a missing destination tail. Never
   normalize away dot segments or follow a reparse point to compare scope. */
static BOOL expanded_path(const WCHAR *input, WCHAR *output) {
    WCHAR *work;
    DWORD length = (DWORD)lstrlenW(input), cut, part = 3, i;
    BOOL result = FALSE;
    if (length < 3 || length >= 32768 || input[1] != L':' || input[2] != L'\\' || !safe_path(input)) return FALSE;
    for (i = 3; i <= length; ++i) {
        if (input[i] == L'/' || input[i] == L':') return FALSE;
        if (input[i] == L'\\' || input[i] == 0) {
            DWORD size = i - part;
            if ((size == 1 && input[part] == L'.') || (size == 2 && input[part] == L'.' && input[part+1] == L'.') || (!size && i < length)) return FALSE;
            part = i + 1;
        }
    }
    work = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, (length + 1) * sizeof(WCHAR));
    if (!work) return FALSE;
    lstrcpyW(work, input);
    cut = length;
    while (cut > 3 && work[cut-1] == L'\\') work[--cut] = 0;
    for (;;) {
        DWORD size = GetLongPathNameW(work, output, 32768), error = GetLastError();
        if (size > 0 && size < 32768) {
            result = append_text(output, 32768, input + cut);
            if (result) {
                size = (DWORD)lstrlenW(output);
                while (size > 3 && output[size-1] == L'\\') output[--size] = 0;
            }
            break;
        }
        if (size || (error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND) || cut <= 3) break;
        while (cut > 3 && work[cut-1] != L'\\') --cut;
        if (cut > 3) --cut;
        work[cut] = 0;
    }
    HeapFree(GetProcessHeap(), 0, work);
    return result;
}
static BOOL same_target_path(const WCHAR *left, const WCHAR *right) {
    WCHAR *buffer = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, 2 * 32768 * sizeof(WCHAR));
    BOOL result;
    if (!buffer) return FALSE;
    result = expanded_path(left, buffer) && expanded_path(right, buffer + 32768) && !lstrcmpiW(buffer, buffer + 32768);
    HeapFree(GetProcessHeap(), 0, buffer);
    return result;
}

#endif
