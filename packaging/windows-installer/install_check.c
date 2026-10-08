/* Read-only preflight shared by the Tauri NSIS installer and uninstaller.
   No process termination, installation, registry write or data cleanup. */
#include "native.h"
#include "guard_paths.h"
#include "os_version.h"
#include "inventory.h"
#include <msi.h>

static WCHAR root[32768], path[32768];

static DWORD check_paths(void) {
    PWSTR local = NULL;
    DWORD index;
    if (!windows10_or_later()) return ERROR_OLD_WIN_VERSION;
    if (FAILED(SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, NULL, &local)))
        return ERROR_INSTALL_FAILURE;
    root[0] = 0;
    if (!append_text(root, 32768, local) || !append_text(root, 32768, L"\\Programs\\Nexa\\")) {
        CoTaskMemFree(local); return ERROR_INSTALL_FAILURE;
    }
    CoTaskMemFree(local);
    if (!same_target_path(root, root) || !safe_path(root) || !stopped(root)) return ERROR_INSTALL_FAILURE;
    for (index = 0; index < NEXA_OWNED_COUNT; ++index) {
        lstrcpyW(path, root);
        if (!append_text(path, 32768, NEXA_OWNED_PATHS[index]) || !safe_path(path)) return ERROR_INSTALL_FAILURE;
    }
    if (FAILED(SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DONT_VERIFY, NULL, &local)))
        return ERROR_INSTALL_FAILURE;
    path[0] = 0;
    if (!append_text(path, 32768, local) || !append_text(path, 32768, L"\\Nexa\\Nexa.lnk")) {
        CoTaskMemFree(local); return ERROR_INSTALL_FAILURE;
    }
    CoTaskMemFree(local);
    return safe_path(path) ? ERROR_SUCCESS : ERROR_INSTALL_FAILURE;
}

void WINAPI CheckEntry(void) {
    int count;
    LPWSTR *arguments = CommandLineToArgvW(GetCommandLineW(), &count);
    DWORD result;
    BOOL nsis;
    WCHAR product[39];
    if (!arguments) ExitProcess(ERROR_NOT_ENOUGH_MEMORY);
    if (count != 2 || (lstrcmpW(arguments[1], L"/check") && lstrcmpW(arguments[1], L"/nsis"))) {
        LocalFree(arguments); ExitProcess(ERROR_INVALID_PARAMETER);
    }
    nsis = !lstrcmpW(arguments[1], L"/nsis");
    LocalFree(arguments);
    result = check_paths();
    if (!result && nsis) {
        /* Includes current-user MSI products installed by the old Setup EXE.
           Do not run their uninstall command or overwrite their owned files. */
        result = MsiEnumRelatedProductsW(L"{85615F8B-FD70-53D9-86ED-4164379CBC40}", 0, 0, product);
        if (result == ERROR_SUCCESS) result = ERROR_PRODUCT_VERSION;
        else if (result == ERROR_NO_MORE_ITEMS) result = ERROR_SUCCESS;
    }
    ExitProcess(result);
}
