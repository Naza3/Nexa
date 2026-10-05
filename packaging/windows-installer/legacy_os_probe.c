/* Private early MSI-context diagnosis of the retired kernel32-file query.
   Never placed in a production MSI. Reports only bounded numeric facts. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <msi.h>
#include <msiquery.h>
__declspec(dllexport) UINT __stdcall NexaLegacyOsProbe(MSIHANDLE session) {
    WCHAR system_file[512];
    DWORD stage = 1, error = 0, major = 0, minor = 0, build = 0;
    DWORD length = GetSystemDirectoryW(system_file, 480), ignored = 0, size = 0;
    BYTE *data = NULL;
    VS_FIXEDFILEINFO *info = NULL;
    UINT info_size = 0;
    BOOL success = FALSE;
    MSIHANDLE record;
    if (!length || length >= 480) { error = GetLastError(); goto done; }
    lstrcatW(system_file, L"\\kernel32.dll");
    stage = 2; size = GetFileVersionInfoSizeW(system_file, &ignored);
    if (!size) { error = GetLastError(); goto done; }
    stage = 3; data = (BYTE *)HeapAlloc(GetProcessHeap(), 0, size);
    if (!data) { error = ERROR_NOT_ENOUGH_MEMORY; goto done; }
    stage = 4;
    if (!GetFileVersionInfoW(system_file, 0, size, data)) { error = GetLastError(); goto done; }
    stage = 5;
    if (!VerQueryValueW(data, L"\\", (LPVOID *)&info, &info_size)) { error = GetLastError(); goto done; }
    stage = 6;
    if (!info || info_size < sizeof(VS_FIXEDFILEINFO) || info->dwSignature != 0xFEEF04BD) { error = ERROR_INVALID_DATA; goto done; }
    major = HIWORD(info->dwFileVersionMS); minor = LOWORD(info->dwFileVersionMS); build = HIWORD(info->dwFileVersionLS);
    stage = 7;
    if (major < 10) { error = ERROR_OLD_WIN_VERSION; goto done; }
    stage = 0; success = TRUE;
 done:
    if (data) HeapFree(GetProcessHeap(), 0, data);
    record = MsiCreateRecord(6);
    if (!record) return ERROR_INSTALL_FAILURE;
    MsiRecordSetStringW(record, 0, L"NexaLegacyOsProbe stage=[1] success=[2] major=[3] minor=[4] build=[5] error=[6]");
    MsiRecordSetInteger(record, 1, (int)stage); MsiRecordSetInteger(record, 2, success ? 1 : 0);
    MsiRecordSetInteger(record, 3, (int)major); MsiRecordSetInteger(record, 4, (int)minor);
    MsiRecordSetInteger(record, 5, (int)build); MsiRecordSetInteger(record, 6, (int)error);
    MsiProcessMessage(session, INSTALLMESSAGE_INFO, record);
    MsiCloseHandle(record);
    return ERROR_SUCCESS; /* Informational. The production EXE is the mandatory gate. */
}
