/* Private compiler-gate test DLL. Never included in an MSI, EXE or release. */
#include "guard.c"
__declspec(dllexport) BOOL __stdcall NexaTestSameTarget(const WCHAR *left, const WCHAR *right) {
    return same_target_path(left, right);
}
