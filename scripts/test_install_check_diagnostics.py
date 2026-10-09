"""Execute helper control flow with fake Win32 calls; not Windows execution."""
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "packaging/windows-installer/install_check.c"
TEMPLATE = ROOT / "packaging/tauri/windows/installer.nsi"

STUBS = r'''
#include <assert.h>
#include <setjmp.h>
#include <stddef.h>
#include <wchar.h>
typedef unsigned long DWORD;
typedef int BOOL;
typedef wchar_t WCHAR;
typedef WCHAR *PWSTR;
typedef WCHAR *LPWSTR;
#define WINAPI
#define FAILED(v) ((v) < 0)
#define ERROR_SUCCESS 0
#define ERROR_OLD_WIN_VERSION 1150
#define ERROR_INSTALL_FAILURE 1603
#define ERROR_NOT_ENOUGH_MEMORY 8
#define ERROR_INVALID_PARAMETER 87
#define ERROR_PRODUCT_VERSION 1638
#define ERROR_NO_MORE_ITEMS 259
#define KF_FLAG_DONT_VERIFY 0
static int FOLDERID_LocalAppData, FOLDERID_Programs;
static const WCHAR *const NEXA_OWNED_PATHS[] = {L"a.exe", L"b.exe"};
#define NEXA_OWNED_COUNT 2
static int os_ok, directory_fail, directory_calls, append_fail, append_calls;
static int path_fail, path_calls, same_ok, process_ok, argc_value, argv_missing;
static DWORD msi_result, exit_result;
static int msi_calls;
static LPWSTR argv_value[4];
static jmp_buf done;
static int windows10_or_later(void) { return os_ok; }
static int SHGetKnownFolderPath(const int *id, int flag, void *token, PWSTR *out) {
    (void)id; (void)flag; (void)token;
    *out = L"C:\\Users\\test";
    return ++directory_calls == directory_fail ? -1 : 0;
}
static BOOL append_text(WCHAR *out, DWORD capacity, const WCHAR *text) {
    (void)capacity;
    if (++append_calls == append_fail) return 0;
    wcscat(out, text); return 1;
}
static void CoTaskMemFree(void *p) { (void)p; }
static BOOL same_target_path(const WCHAR *a, const WCHAR *b) {
    (void)a; (void)b; return same_ok;
}
static BOOL safe_path(const WCHAR *p) { (void)p; return ++path_calls != path_fail; }
static BOOL stopped(const WCHAR *p) { (void)p; return process_ok; }
static void lstrcpyW(WCHAR *out, const WCHAR *in) { wcscpy(out, in); }
static int lstrcmpW(const WCHAR *a, const WCHAR *b) { return wcscmp(a,b); }
static LPWSTR GetCommandLineW(void) { return L"ignored"; }
static LPWSTR *CommandLineToArgvW(LPWSTR command, int *count) {
    (void)command; *count = argc_value; return argv_missing ? NULL : argv_value;
}
static void LocalFree(void *p) { (void)p; }
static DWORD MsiEnumRelatedProductsW(const WCHAR *code, int a, int b, WCHAR *out) {
    (void)code; (void)a; (void)b; (void)out; ++msi_calls; return msi_result;
}
static _Noreturn void ExitProcess(DWORD code) { exit_result = code; longjmp(done, 1); }
static void reset(void) {
    os_ok = same_ok = process_ok = 1;
    directory_fail = directory_calls = append_fail = append_calls = 0;
    path_fail = path_calls = argv_missing = msi_calls = 0;
    msi_result = ERROR_NO_MORE_ITEMS;
    argc_value = 3;
    argv_value[0] = L"helper"; argv_value[1] = L"/nsis";
    argv_value[2] = L"/diagnostic"; argv_value[3] = NULL;
}
'''

HARNESS = r'''
static DWORD run(void) { if (!setjmp(done)) CheckEntry(); return exit_result; }
int main(void) {
    int diagnostic, i;
    for (diagnostic = 0; diagnostic <= 1; ++diagnostic) {
        reset(); argc_value = 2 + diagnostic; assert(run() == 0); assert(msi_calls == 1);
        reset(); argc_value = 2 + diagnostic; argv_value[1] = L"/check";
        msi_result = 5; assert(run() == 0); assert(msi_calls == 0);
        reset(); argc_value = 2 + diagnostic; os_ok = 0;
        assert(run() == (diagnostic ? NEXA_CHECK_OS : ERROR_OLD_WIN_VERSION));
        assert(msi_calls == 0);
        for (i = 1; i <= 2; ++i) {
            reset(); argc_value = 2 + diagnostic; directory_fail = i;
            assert(run() == (diagnostic ? NEXA_CHECK_DIRECTORY : ERROR_INSTALL_FAILURE));
        }
        for (i = 1; i <= 6; ++i) {
            reset(); argc_value = 2 + diagnostic; append_fail = i;
            assert(run() == (diagnostic ? (i == 3 || i == 4 ? NEXA_CHECK_PATH : NEXA_CHECK_DIRECTORY) : ERROR_INSTALL_FAILURE));
        }
        reset(); argc_value = 2 + diagnostic; same_ok = 0;
        assert(run() == (diagnostic ? NEXA_CHECK_PATH : ERROR_INSTALL_FAILURE));
        for (i = 1; i <= 4; ++i) {
            reset(); argc_value = 2 + diagnostic; path_fail = i;
            assert(run() == (diagnostic ? NEXA_CHECK_PATH : ERROR_INSTALL_FAILURE));
        }
        reset(); argc_value = 2 + diagnostic; process_ok = 0;
        assert(run() == (diagnostic ? NEXA_CHECK_PROCESS : ERROR_INSTALL_FAILURE));
        reset(); argc_value = 2 + diagnostic; msi_result = 0;
        assert(run() == ERROR_PRODUCT_VERSION);
        reset(); argc_value = 2 + diagnostic; msi_result = 5;
        assert(run() == (diagnostic ? NEXA_CHECK_REGISTRY : 5));
        reset(); argc_value = 2 + diagnostic; msi_result = NEXA_CHECK_PROCESS;
        assert(run() == (diagnostic ? NEXA_CHECK_REGISTRY : NEXA_CHECK_PROCESS));
    }
    reset(); argv_missing = 1; assert(run() == ERROR_NOT_ENOUGH_MEMORY);
    reset(); argc_value = 1; assert(run() == ERROR_INVALID_PARAMETER);
    reset(); argc_value = 4; assert(run() == ERROR_INVALID_PARAMETER);
    reset(); argv_value[1] = L"/invalid"; assert(run() == ERROR_INVALID_PARAMETER);
    reset(); argv_value[2] = L"/invalid"; assert(run() == ERROR_INVALID_PARAMETER);
    return 0;
}
'''


class InstallCheckDiagnosticsTests(unittest.TestCase):
    def test_helper_control_flow_and_legacy_exit_contract(self):
        compiler = shutil.which("cc") or shutil.which("clang")
        if not compiler:
            self.skipTest("host C compiler unavailable; Windows execution is a separate gate")
        source = re.sub(r'^#include .*\n', '', HELPER.read_text(encoding="utf-8"), flags=re.M)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "check.c").write_text(STUBS + source + HARNESS, encoding="utf-8")
            subprocess.run([compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", str(path / "check.c"),
                            "-o", str(path / "check")], check=True, capture_output=True, text=True, encoding="utf-8")
            subprocess.run([str(path / "check")], check=True, capture_output=True, text=True, encoding="utf-8")

    def test_nsis_allowlisted_diagnostics_and_external_contract(self):
        source = TEMPLATE.read_text(encoding="utf-8")
        check = source.split('Function ${prefix}NexaCheck\n', 1)[1].split('FunctionEnd', 1)[0]
        self.assertIn('${mode} /diagnostic', check)
        self.assertEqual(set(re.findall(r'SetErrorLevel (\d+)', check)), {'1603', '1638'})
        reasons = {20001: 'unsupported_os', 20002: 'directory_lookup_failed',
                   20003: 'path_verification_failed', 20004: 'process_verification_failed',
                   20005: 'msi_inventory_failed'}
        for code, reason in reasons.items():
            branch = check.split('$NexaCheckResult = ' + str(code), 1)[1].split('${Else', 1)[0]
            self.assertIn('DetailPrint "Nexa preflight: ' + reason + '"', branch)
            self.assertIn('[' + reason + ']', branch)
            self.assertRegex(branch, r'[\u4e00-\u9fff]')
            self.assertNotIn('$INSTDIR', branch)
            self.assertNotIn('Stop Nexa', branch if code != 20004 else '')
        self.assertIn('or Windows could not verify its process state', check)
        self.assertIn('Nexa preflight: check_failed', check)
        self.assertIn('Nexa preflight: helper_launch_failed', check)
        self.assertNotIn('Models and user data have not been removed', check)


if __name__ == '__main__':
    unittest.main()
