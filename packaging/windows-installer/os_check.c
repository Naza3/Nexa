/* Mandatory MSI Type 2 pre-transaction guard, with the same Win10 manifest and
   version predicate as Setup. No UI, arguments, mutation or fallback bypass. */
#include "os_version.h"
void WINAPI OsCheckEntry(void) {
    DWORD error;
    if (windows10_or_later()) ExitProcess(ERROR_SUCCESS);
    error = GetLastError();
    ExitProcess(error ? error : ERROR_OLD_WIN_VERSION);
}
