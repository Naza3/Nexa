/* Small offline wizard around one embedded MSI. Windows Installer owns the
   entire install/repair/remove transaction. No copied application files here. */
#include "native.h"
#include "os_version.h"
#include "setup_identity.h"
#include <bcrypt.h>
#include <commctrl.h>
#include <sddl.h>

#define ID_INSTALL 1001
#define ID_REPAIR 1002
#define ID_REMOVE 1003
#define ID_NEXT 1004
#define ID_CANCEL 1005
#define ID_BACK 1006
#define WM_INSTALL_DONE (WM_APP + 1)
static HWND window, body, install_choice, repair_choice, remove_choice, next_button, cancel_button, back_button, progress;
static HANDLE msi_file = INVALID_HANDLE_VALUE;
static WCHAR temporary_dir[32768], msi_path[32768], command_line[32768], system_exe[32768];
static DWORD result_code = ERROR_INSTALL_USEREXIT;
static int action = ID_INSTALL, page = 0;
static BOOL running = FALSE, silent = FALSE;
static HINSTANCE instance;

static BOOL valid_hash(const BYTE *bytes, DWORD size) {
    BCRYPT_ALG_HANDLE algorithm = NULL;
    BCRYPT_HASH_HANDLE hash = NULL;
    BYTE digest[32];
    char hex[65];
    DWORD i;
    BOOL ok = FALSE;
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, NULL, 0) < 0) return FALSE;
    if (BCryptCreateHash(algorithm, &hash, NULL, 0, NULL, 0, 0) >= 0) {
        if (BCryptHashData(hash, (PUCHAR)bytes, size, 0) >= 0 && BCryptFinishHash(hash, digest, 32, 0) >= 0) {
            for (i = 0; i < 32; ++i) {
                hex[i*2] = "0123456789abcdef"[digest[i] >> 4];
                hex[i*2+1] = "0123456789abcdef"[digest[i] & 15];
            }
            hex[64] = 0;
            ok = !lstrcmpA(hex, NEXA_MSI_SHA256);
        }
        BCryptDestroyHash(hash);
    }
    BCryptCloseAlgorithmProvider(algorithm, 0);
    return ok;
}
static void cleanup(void) {
    if (msi_file != INVALID_HANDLE_VALUE) { CloseHandle(msi_file); msi_file = INVALID_HANDLE_VALUE; }
    if (msi_path[0]) DeleteFileW(msi_path);
    if (temporary_dir[0]) RemoveDirectoryW(temporary_dir);
    /* No recursion and no cleanup of the application's installation/data dirs. */
}
static DWORD extract_msi(void) {
    HRSRC resource = FindResourceW(instance, MAKEINTRESOURCEW(101), RT_RCDATA);
    HGLOBAL loaded;
    const BYTE *bytes;
    BYTE random[16], *readback;
    WCHAR suffix[33];
    DWORD size, count, i, length;
    PSECURITY_DESCRIPTOR descriptor = NULL;
    SECURITY_ATTRIBUTES security;
    if (!resource) return ERROR_INVALID_DATA;
    loaded = LoadResource(instance, resource);
    bytes = loaded ? (const BYTE *)LockResource(loaded) : NULL;
    size = SizeofResource(instance, resource);
    if (!bytes || !size || !valid_hash(bytes, size)) return ERROR_CRC;
    length = GetTempPathW(32700, temporary_dir);
    if (!length || length >= 32700 || !safe_path(temporary_dir)) { temporary_dir[0] = 0; return ERROR_BAD_PATHNAME; }
    if (BCryptGenRandom(NULL, random, 16, BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0) { temporary_dir[0] = 0; return ERROR_GEN_FAILURE; }
    for (i = 0; i < 16; ++i) {
        suffix[i*2] = L"0123456789abcdef"[random[i] >> 4];
        suffix[i*2+1] = L"0123456789abcdef"[random[i] & 15];
    }
    suffix[32] = 0;
    if (!append_text(temporary_dir, 32768, L"NexaSetup-") || !append_text(temporary_dir, 32768, suffix)) { temporary_dir[0] = 0; return ERROR_BAD_PATHNAME; }
    /* Fresh unpredictable folder; only its owner and SYSTEM inherit access. */
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(L"D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)", SDDL_REVISION_1, &descriptor, NULL)) { temporary_dir[0] = 0; return ERROR_ACCESS_DENIED; }
    zero_bytes(&security, sizeof(security));
    security.nLength = sizeof(security); security.lpSecurityDescriptor = descriptor;
    if (!CreateDirectoryW(temporary_dir, &security)) {
        DWORD error = GetLastError();
        LocalFree(descriptor); temporary_dir[0] = 0;
        return error;
    }
    LocalFree(descriptor);
    lstrcpyW(msi_path, temporary_dir);
    if (!append_text(msi_path, 32768, L"\\Nexa.msi")) return ERROR_BAD_PATHNAME;
    msi_file = CreateFileW(msi_path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
    if (msi_file == INVALID_HANDLE_VALUE) return GetLastError();
    if (!WriteFile(msi_file, bytes, size, &count, NULL) || count != size || !FlushFileBuffers(msi_file)) return ERROR_WRITE_FAULT;
    readback = (BYTE *)HeapAlloc(GetProcessHeap(), 0, size);
    if (!readback) return ERROR_NOT_ENOUGH_MEMORY;
    SetFilePointer(msi_file, 0, NULL, FILE_BEGIN);
    if (!ReadFile(msi_file, readback, size, &count, NULL) || count != size || !valid_hash(readback, size)) {
        HeapFree(GetProcessHeap(), 0, readback); return ERROR_CRC;
    }
    HeapFree(GetProcessHeap(), 0, readback);
    CloseHandle(msi_file);
    msi_file = CreateFileW(msi_path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (msi_file == INVALID_HANDLE_VALUE) return GetLastError();
    readback = (BYTE *)HeapAlloc(GetProcessHeap(), 0, size);
    if (!readback) return ERROR_NOT_ENOUGH_MEMORY;
    if (!ReadFile(msi_file, readback, size, &count, NULL) || count != size || !valid_hash(readback, size)) {
        HeapFree(GetProcessHeap(), 0, readback); return ERROR_CRC;
    }
    HeapFree(GetProcessHeap(), 0, readback);
    /* Keep the handle open (no write/delete sharing) until msiexec exits. */
    return ERROR_SUCCESS;
}
static DWORD perform_install(void) {
    STARTUPINFOW startup;
    PROCESS_INFORMATION process;
    DWORD code = extract_msi(), length;
    if (code != ERROR_SUCCESS) { cleanup(); return code; }
    length = GetSystemDirectoryW(system_exe, 32750);
    if (!length || length >= 32750 || !append_text(system_exe, 32768, L"\\msiexec.exe")) { cleanup(); return ERROR_BAD_PATHNAME; }
    command_line[0] = 0;
    if (!append_text(command_line, 32768, L"\"") || !append_text(command_line, 32768, system_exe) ||
        !append_text(command_line, 32768, action == ID_REMOVE ? L"\" /x \"" : action == ID_REPAIR ? L"\" /fa \"" : L"\" /i \"") ||
        !append_text(command_line, 32768, msi_path) || !append_text(command_line, 32768, silent ? L"\" /qn /norestart REBOOT=ReallySuppress" : L"\" /qb /norestart REBOOT=ReallySuppress")) {
        cleanup(); return ERROR_BAD_PATHNAME;
    }
    zero_bytes(&startup, sizeof(startup)); zero_bytes(&process, sizeof(process)); startup.cb = sizeof(startup);
    if (!CreateProcessW(system_exe, command_line, NULL, NULL, FALSE, 0, NULL, temporary_dir, &startup, &process)) {
        code = GetLastError(); cleanup(); return code;
    }
    CloseHandle(process.hThread);
    /* Never kill/abandon the installer. Its own Cancel performs rollback. */
    if (WaitForSingleObject(process.hProcess, INFINITE) != WAIT_OBJECT_0 || !GetExitCodeProcess(process.hProcess, &code)) code = ERROR_GEN_FAILURE;
    CloseHandle(process.hProcess);
    cleanup();
    return code; /* Includes 0, 1602, 3010 and every error unchanged. */
}
static DWORD WINAPI worker(LPVOID unused) {
    DWORD code;
    (void)unused;
    code = perform_install();
    PostMessageW(window, WM_INSTALL_DONE, (WPARAM)code, 0);
    return code;
}
static void show_page(void) {
    BOOL choose = page == 0;
    ShowWindow(install_choice, choose ? SW_SHOW : SW_HIDE);
    ShowWindow(repair_choice, choose ? SW_SHOW : SW_HIDE);
    ShowWindow(remove_choice, choose ? SW_SHOW : SW_HIDE);
    ShowWindow(back_button, page == 1 ? SW_SHOW : SW_HIDE);
    ShowWindow(progress, page == 2 ? SW_SHOW : SW_HIDE);
    EnableWindow(cancel_button, page != 2);
    EnableWindow(next_button, page != 2);
    SetWindowTextW(next_button, page == 0 ? L"Next >" : page == 1 ? L"Apply" : L"Finish");
    if (page == 0) SetWindowTextW(body,
        L"Welcome to Nexa Setup\r\n\r\nChoose an action for the current Windows user.\r\nThe MSI and this wizard manage the same Nexa installation.");
    if (page == 1) SetWindowTextW(body,
        action == ID_REMOVE ?
        L"Ready to remove Nexa\r\n\r\nStop the Nexa service and close its window first.\r\nOnly installer-owned program files and shortcuts are removed.\r\nModels, settings, keys and external files are kept.\r\n\r\nChoose Apply to continue, or Back to change the action." :
        L"Ready to install or repair Nexa\r\n\r\nLocation: %LOCALAPPDATA%\\Programs\\Nexa\r\nStop the Nexa service and close its window first.\r\nMicrosoft Edge WebView2 Evergreen Runtime must already be installed.\r\nIf missing, obtain it from https://developer.microsoft.com/microsoft-edge/webview2/\r\nSetup does not download or install prerequisites.\r\nThis package is unsigned. Use a trusted release and verify SHA-256.\r\n\r\nChoose Apply to continue. Your models, settings and keys are kept.");
    if (page == 2) {
        SetWindowTextW(body, L"Windows Installer is applying your selection...\r\n\r\nUse Cancel in the Windows Installer progress window to cancel safely.\r\nKeep this window open until Windows Installer finishes or rolls back.\r\nYour computer will not restart automatically.");
        SendMessageW(progress, PBM_SETMARQUEE, TRUE, 30);
    }
}
static HWND control(const WCHAR *klass, const WCHAR *text, DWORD style, int x, int y, int width, int height, int id) {
    HWND child = CreateWindowExW(0, klass, text, WS_CHILD | WS_VISIBLE | style, x, y, width, height, window, (HMENU)(INT_PTR)id, instance, NULL);
    if (child) SendMessageW(child, WM_SETFONT, (WPARAM)GetStockObject(DEFAULT_GUI_FONT), TRUE);
    return child;
}
static LRESULT CALLBACK window_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
    switch (message) {
    case WM_CREATE:
        window = hwnd;
        body = control(L"STATIC", L"", SS_LEFT, 24, 20, 620, 230, 1100);
        install_choice = control(L"BUTTON", L"Install / upgrade", BS_AUTORADIOBUTTON | WS_GROUP | WS_TABSTOP, 32, 170, 220, 28, ID_INSTALL);
        repair_choice = control(L"BUTTON", L"Repair this version", BS_AUTORADIOBUTTON | WS_TABSTOP, 32, 208, 220, 28, ID_REPAIR);
        remove_choice = control(L"BUTTON", L"Remove this version", BS_AUTORADIOBUTTON | WS_TABSTOP, 32, 246, 220, 28, ID_REMOVE);
        back_button = control(L"BUTTON", L"< Back", BS_PUSHBUTTON | WS_TABSTOP, 314, 330, 100, 32, ID_BACK);
        next_button = control(L"BUTTON", L"Next >", BS_DEFPUSHBUTTON | WS_TABSTOP, 424, 330, 100, 32, ID_NEXT);
        cancel_button = control(L"BUTTON", L"Cancel", BS_PUSHBUTTON | WS_TABSTOP, 534, 330, 100, 32, ID_CANCEL);
        progress = control(PROGRESS_CLASSW, L"", PBS_MARQUEE, 24, 272, 610, 24, 1101);
        SendMessageW(action == ID_REPAIR ? repair_choice : action == ID_REMOVE ? remove_choice : install_choice, BM_SETCHECK, BST_CHECKED, 0);
        show_page();
        return 0;
    case WM_COMMAND:
        switch (LOWORD(wparam)) {
        case ID_INSTALL: case ID_REPAIR: case ID_REMOVE: action = LOWORD(wparam); return 0;
        case ID_BACK: if (page == 1) { page = 0; show_page(); } return 0;
        case ID_NEXT:
            if (page == 0) { page = 1; show_page(); }
            else if (page == 1) {
                HANDLE thread;
                page = 2; running = TRUE; show_page();
                thread = CreateThread(NULL, 0, worker, NULL, 0, NULL);
                if (thread) CloseHandle(thread);
                else PostMessageW(hwnd, WM_INSTALL_DONE, ERROR_NOT_ENOUGH_MEMORY, 0);
            } else if (page == 3) DestroyWindow(hwnd);
            return 0;
        case ID_CANCEL:
            if (!running) DestroyWindow(hwnd);
            return 0;
        }
        break;
    case WM_INSTALL_DONE:
        running = FALSE; page = 3; result_code = (DWORD)wparam; show_page();
        ShowWindow(cancel_button, SW_HIDE);
        if (result_code == ERROR_SUCCESS) SetWindowTextW(body, L"Nexa Setup completed successfully.\r\n\r\nYou may close this wizard. Your models and user data were kept.");
        else if (result_code == ERROR_SUCCESS_REBOOT_REQUIRED) SetWindowTextW(body, L"Nexa Setup completed. Windows reports that a restart is required.\r\n\r\nSave your work and restart when convenient. Setup will not restart automatically.");
        else if (result_code == ERROR_INSTALL_USEREXIT) SetWindowTextW(body, L"Setup was canceled. Windows Installer has finished its rollback.\r\n\r\nYou may close this wizard and retry later.");
        else {
            WCHAR text[256];
            wsprintfW(text, L"Nexa Setup did not complete (Windows Installer code %lu).\r\n\r\nStop the Nexa service, close the desktop window and retry.\r\nFor repair, use the installer matching your installed version.\r\nDo not delete your models or user data.", result_code);
            SetWindowTextW(body, text);
        }
        return 0;
    case WM_CLOSE:
        if (running) {
            MessageBoxW(hwnd, L"Windows Installer is still running. Use Cancel in its progress window and wait for rollback before closing Setup.", L"Nexa Setup", MB_OK | MB_ICONINFORMATION);
        } else DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY: PostQuitMessage(0); return 0;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}
void WINAPI SetupEntry(void) {
    int argc, index;
    LPWSTR *argv;
    WNDCLASSEXW klass;
    MSG message;
    INITCOMMONCONTROLSEX controls;
    BOOL valid = TRUE, action_seen = FALSE;
    instance = GetModuleHandleW(NULL);
    argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (!argv) ExitProcess(ERROR_NOT_ENOUGH_MEMORY);
    for (index = 1; index < argc; ++index) {
        if (!lstrcmpiW(argv[index], L"/S")) silent = TRUE;
        else if (!action_seen && (!lstrcmpiW(argv[index], L"/install") || !lstrcmpiW(argv[index], L"/repair") || !lstrcmpiW(argv[index], L"/uninstall"))) {
            action_seen = TRUE;
            action = !lstrcmpiW(argv[index], L"/repair") ? ID_REPAIR : !lstrcmpiW(argv[index], L"/uninstall") ? ID_REMOVE : ID_INSTALL;
        } else valid = FALSE;
    }
    LocalFree(argv);
    if (!valid) ExitProcess(ERROR_INVALID_PARAMETER);
    if (!windows10_or_later()) ExitProcess(ERROR_OLD_WIN_VERSION);
    if (silent) ExitProcess(perform_install());
    zero_bytes(&controls, sizeof(controls)); controls.dwSize = sizeof(controls); controls.dwICC = ICC_PROGRESS_CLASS;
    InitCommonControlsEx(&controls);
    zero_bytes(&klass, sizeof(klass)); klass.cbSize = sizeof(klass); klass.lpfnWndProc = window_proc;
    klass.hInstance = instance; klass.hCursor = LoadCursorW(NULL, IDC_ARROW); klass.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1); klass.lpszClassName = L"NexaSetupWizard";
    if (!RegisterClassExW(&klass)) ExitProcess(GetLastError());
    window = CreateWindowExW(WS_EX_CONTROLPARENT, klass.lpszClassName, L"Nexa Setup", WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, CW_USEDEFAULT, CW_USEDEFAULT, 680, 420, NULL, NULL, instance, NULL);
    if (!window) ExitProcess(GetLastError());
    ShowWindow(window, SW_SHOW); UpdateWindow(window);
    while (GetMessageW(&message, NULL, 0, 0) > 0) {
        if (!IsDialogMessageW(window, &message)) { TranslateMessage(&message); DispatchMessageW(&message); }
    }
    ExitProcess(result_code);
}
