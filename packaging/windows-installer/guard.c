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
#include "guard_paths.h"

static UINT check_session(MSIHANDLE session, WCHAR *root, WCHAR *expected, WCHAR *path) {
    WCHAR scope[16];
    PWSTR local = NULL;
    MSIHANDLE database = 0, view = 0, row = 0;
    UINT code;
    DWORD length;
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

/* Commit-only uninstall cleanup. Updates retain the user's startup choice.
   Never remove another Nexa copy's entry or the containing Run registry key. */
static LSTATUS matching_autostart(HKEY key, const WCHAR *expected, WCHAR *actual, BOOL *matches) {
    DWORD type = 0, size = 32768 * sizeof(WCHAR);
    LSTATUS result = RegQueryValueExW(key, L"Nexa", NULL, &type, (BYTE *)actual, &size);
    *matches = FALSE;
    /* Malformed/oversized or foreign commands are not owned by this installer. */
    if (result == ERROR_MORE_DATA || result == ERROR_FILE_NOT_FOUND) return ERROR_SUCCESS;
    if (result == ERROR_SUCCESS && type == REG_SZ &&
        size == ((DWORD)lstrlenW(expected) + 1) * sizeof(WCHAR) &&
        actual[size / sizeof(WCHAR) - 1] == 0 && !lstrcmpiW(actual, expected)) *matches = TRUE;
    return result;
}

__declspec(dllexport) UINT __stdcall NexaRemoveAutostart(MSIHANDLE session) {
    PWSTR local = NULL;
    WCHAR *buffer, *expected, *actual;
    HKEY user, key;
    BOOL matches = FALSE;
    LSTATUS result;
    (void)session;
    if (FAILED(SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, NULL, &local)))
        return ERROR_INSTALL_FAILURE;
    buffer = (WCHAR *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, 2 * 32768 * sizeof(WCHAR));
    if (!buffer) { CoTaskMemFree(local); return ERROR_NOT_ENOUGH_MEMORY; }
    expected = buffer; actual = buffer + 32768;
    if (!append_text(expected, 32768, L"\"") || !append_text(expected, 32768, local) ||
        !append_text(expected, 32768, L"\\Programs\\Nexa\\nexa-desktop.exe\"")) {
        HeapFree(GetProcessHeap(), 0, buffer); CoTaskMemFree(local); return ERROR_INSTALL_FAILURE;
    }
    CoTaskMemFree(local);
    /* Resolve the impersonated current user explicitly in the MSI service. */
    result = RegOpenCurrentUser(KEY_QUERY_VALUE, &user);
    if (result == ERROR_SUCCESS) {
        result = RegOpenKeyExW(user, L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                              0, KEY_QUERY_VALUE, &key);
        if (result == ERROR_SUCCESS) {
            result = matching_autostart(key, expected, actual, &matches);
            RegCloseKey(key);
        }
        if (result == ERROR_SUCCESS && matches) {
            result = RegOpenKeyExW(user, L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                                  0, KEY_QUERY_VALUE | KEY_SET_VALUE, &key);
            if (result == ERROR_SUCCESS) {
                /* Recheck after acquiring write access; preserve replaced values. */
                result = matching_autostart(key, expected, actual, &matches);
                if (result == ERROR_SUCCESS && matches) result = RegDeleteValueW(key, L"Nexa");
                RegCloseKey(key);
            }
        }
        RegCloseKey(user);
    }
    HeapFree(GetProcessHeap(), 0, buffer);
    return result == ERROR_FILE_NOT_FOUND ? ERROR_SUCCESS : (UINT)result;
}
