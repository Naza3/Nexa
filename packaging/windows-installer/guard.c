/* Read-only MSI guard. Never stops a process or changes user data/settings. */
#include "native.h"
#include <msi.h>
#include <msiquery.h>
#include <tlhelp32.h>

static UINT refuse(MSIHANDLE session, const WCHAR *message) {
    MSIHANDLE record = MsiCreateRecord(0);
    if (record) {
        MsiRecordSetStringW(record, 0, message);
        MsiProcessMessage(session, INSTALLMESSAGE_ERROR, record);
        MsiCloseHandle(record);
    }
    return ERROR_INSTALL_FAILURE;
}
static BOOL get_property(MSIHANDLE session, const WCHAR *name, WCHAR *value, DWORD capacity) {
    DWORD size = capacity;
    return MsiGetPropertyW(session, name, value, &size) == ERROR_SUCCESS;
}
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
/* MSI's VersionNT is 603 even on Windows 10. Read the inbox kernel32 version
   resource instead of relying on that compatibility-virtualized property. */
static BOOL windows10_kernel(void) {
    WCHAR system_file[512];
    DWORD length = GetSystemDirectoryW(system_file, 480), ignored = 0, size;
    BYTE *data;
    VS_FIXEDFILEINFO *info = NULL;
    UINT info_size = 0;
    BOOL result = FALSE;
    if (!length || length >= 480 || !append_text(system_file, 512, L"\\kernel32.dll")) return FALSE;
    size = GetFileVersionInfoSizeW(system_file, &ignored);
    if (!size) return FALSE;
    data = (BYTE *)HeapAlloc(GetProcessHeap(), 0, size);
    if (!data) return FALSE;
    if (GetFileVersionInfoW(system_file, 0, size, data) && VerQueryValueW(data, L"\\", (LPVOID *)&info, &info_size) &&
        info_size >= sizeof(VS_FIXEDFILEINFO) && info->dwSignature == 0xFEEF04BD && HIWORD(info->dwFileVersionMS) >= 10) result = TRUE;
    HeapFree(GetProcessHeap(), 0, data);
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

static UINT check_session(MSIHANDLE session, WCHAR *root, WCHAR *expected, WCHAR *path) {
    WCHAR scope[16];
    PWSTR local = NULL;
    MSIHANDLE database = 0, view = 0, row = 0;
    UINT code;
    DWORD length;
    if (!windows10_kernel()) return refuse(session, L"Nexa requires Windows 10 or later (64-bit). Windows version could not be confirmed.");
    if (!get_property(session, L"ALLUSERS", scope, 16) || scope[0])
        return refuse(session, L"Nexa only supports a current-user installation.");
    if (FAILED(SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, NULL, &local)))
        return refuse(session, L"Cannot resolve the current user's local application directory.");
    expected[0] = 0;
    if (!append_text(expected, 32768, local) || !append_text(expected, 32768, L"\\Programs\\Nexa\\")) {
        CoTaskMemFree(local);
        return refuse(session, L"Installation path is too long.");
    }
    CoTaskMemFree(local);
    if (!get_property(session, L"INSTALLFOLDER", root, 32768))
        return refuse(session, L"Cannot resolve the installation directory.");
    length = (DWORD)lstrlenW(root);
    if (!length || (root[length-1] != L'\\' && !append_text(root, 32768, L"\\")) || !same_target_path(root, expected))
        return refuse(session, L"Nexa must be installed in the current user's LocalAppData\\Programs\\Nexa directory.");
    if (!safe_path(root))
        return refuse(session, L"Nexa installation path is inaccessible or contains a symbolic link/reparse point. No files were changed.");
    database = MsiGetActiveDatabase(session);
    if (!database) return refuse(session, L"Cannot read the Nexa installation inventory.");
    code = MsiDatabaseOpenViewW(database, L"SELECT `Path` FROM `NexaPayload`", &view);
    if (code == ERROR_SUCCESS) code = MsiViewExecute(view, 0);
    while (code == ERROR_SUCCESS) {
        WCHAR relative[256]; DWORD size = 256;
        code = MsiViewFetch(view, &row);
        if (code != ERROR_SUCCESS) break;
        code = MsiRecordGetStringW(row, 1, relative, &size);
        MsiCloseHandle(row); row = 0;
        lstrcpyW(path, root);
        if (code != ERROR_SUCCESS || !append_text(path, 32768, relative) || !safe_path(path)) {
            code = ERROR_INSTALL_FAILURE; break;
        }
    }
    if (view) MsiCloseHandle(view);
    MsiCloseHandle(database);
    if (code != ERROR_NO_MORE_ITEMS)
        return refuse(session, L"Nexa managed files are inaccessible or redirected by a symbolic link/reparse point. No files were changed.");
    if (FAILED(SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DONT_VERIFY, NULL, &local)))
        return refuse(session, L"Cannot resolve the current user's Start Menu directory.");
    expected[0] = 0;
    if (!append_text(expected, 32768, local) || !append_text(expected, 32768, L"\\Nexa\\")) {
        CoTaskMemFree(local); return refuse(session, L"Start Menu path is too long.");
    }
    CoTaskMemFree(local);
    if (!get_property(session, L"NexaMenuFolder", path, 32768))
        return refuse(session, L"Cannot resolve the Nexa Start Menu directory.");
    length = (DWORD)lstrlenW(path);
    if (!length || (path[length-1] != L'\\' && !append_text(path, 32768, L"\\")) || !same_target_path(path, expected) ||
        !safe_path(path) || !append_text(path, 32768, L"Nexa.lnk") || !safe_path(path))
        return refuse(session, L"Nexa Start Menu path is inaccessible, outside the user profile or redirected by a link. No files were changed.");
    if (!stopped(root))
        return refuse(session, L"Nexa is running or its process state could not be verified. Stop the service in Nexa, close its window, then retry. Setup never force-stops processes.");
    return ERROR_SUCCESS;
}

__declspec(dllexport) UINT __stdcall NexaGuard(MSIHANDLE session) {
    WCHAR *buffer = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, 3 * 32768 * sizeof(WCHAR));
    UINT result;
    if (!buffer) return refuse(session, L"Cannot allocate installer validation memory.");
    result = check_session(session, buffer, buffer + 32768, buffer + 65536);
    HeapFree(GetProcessHeap(), 0, buffer);
    return result;
}
