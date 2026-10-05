"""Small typed wrapper around the Windows inbox MSI database API (no COM/SDK download)."""
from __future__ import annotations
import ctypes
from contextlib import contextmanager
from pathlib import Path
import sys


class Msi:
    def __init__(self):
        if sys.platform != "win32" or sys.maxsize <= 2**32:
            raise ValueError("native Windows x64 Python is required")
        self.dll = ctypes.WinDLL("msi.dll", use_last_error=True)
        U, S, P = ctypes.c_uint, ctypes.c_wchar_p, ctypes.c_void_p
        self.bind("MsiOpenDatabaseW", [S, P, ctypes.POINTER(U)])
        self.bind("MsiCloseHandle", [U])
        self.bind("MsiDatabaseOpenViewW", [U, S, ctypes.POINTER(U)])
        self.bind("MsiViewExecute", [U, U])
        self.bind("MsiViewFetch", [U, ctypes.POINTER(U)])
        self.bind("MsiCreateRecord", [U])
        self.bind("MsiRecordSetStringW", [U, U, S])
        self.bind("MsiRecordSetInteger", [U, U, ctypes.c_int])
        self.bind("MsiRecordSetStreamW", [U, U, S])
        self.bind("MsiRecordReadStream", [U, U, P, ctypes.POINTER(U)])
        self.bind("MsiRecordGetStringW", [U, U, S, ctypes.POINTER(U)])
        self.bind("MsiRecordGetFieldCount", [U])
        self.bind("MsiDatabaseCommit", [U])
        self.bind("MsiGetSummaryInformationW", [U, S, U, ctypes.POINTER(U)])
        self.bind("MsiSummaryInfoSetPropertyW", [U, U, U, ctypes.c_int, P, S])
        self.bind("MsiSummaryInfoPersist", [U])
        self.bind("MsiVerifyPackageW", [S])
        self.bind("MsiQueryProductStateW", [S], ctypes.c_int)

    def bind(self, name, args, result=ctypes.c_uint):
        function = getattr(self.dll, name)
        function.argtypes, function.restype = args, result
        setattr(self, name, function)

    def check(self, code, action):
        if code:
            raise ValueError(f"Windows Installer {action} failed: {code}")

    @contextmanager
    def database(self, path, mode=0):
        handle = ctypes.c_uint()
        self.check(self.MsiOpenDatabaseW(str(path), ctypes.c_void_p(mode), ctypes.byref(handle)), "open database")
        try:
            yield handle.value
        finally:
            self.MsiCloseHandle(handle.value)

    @contextmanager
    def view(self, database, sql):
        handle = ctypes.c_uint()
        self.check(self.MsiDatabaseOpenViewW(database, sql, ctypes.byref(handle)), "open view: " + sql)
        try:
            yield handle.value
        finally:
            self.MsiCloseHandle(handle.value)

    def execute(self, database, sql, values=()):
        record = self.MsiCreateRecord(len(values))
        if not record:
            raise ValueError("Windows Installer could not allocate a record")
        try:
            for index, value in enumerate(values, 1):
                if value is None:
                    continue
                if isinstance(value, Path):
                    code = self.MsiRecordSetStreamW(record, index, str(value))
                elif isinstance(value, int):
                    code = self.MsiRecordSetInteger(record, index, value)
                else:
                    code = self.MsiRecordSetStringW(record, index, str(value))
                self.check(code, "set record")
            with self.view(database, sql) as view:
                self.check(self.MsiViewExecute(view, record), "execute: " + sql)
        finally:
            self.MsiCloseHandle(record)

    def rows(self, database, sql):
        result = []
        with self.view(database, sql) as view:
            self.check(self.MsiViewExecute(view, 0), "query")
            while True:
                record = ctypes.c_uint()
                code = self.MsiViewFetch(view, ctypes.byref(record))
                if code == 259:
                    break
                self.check(code, "fetch")
                try:
                    row = []
                    for field in range(1, self.MsiRecordGetFieldCount(record.value) + 1):
                        size = ctypes.c_uint(32768)
                        buffer = ctypes.create_unicode_buffer(size.value)
                        self.check(self.MsiRecordGetStringW(record.value, field, buffer, ctypes.byref(size)), "read field")
                        row.append(buffer.value)
                    result.append(row)
                finally:
                    self.MsiCloseHandle(record.value)
        return result

    def stream(self, database, name):
        # Names below are generated identifiers, not user-controlled SQL.
        if not name.replace("_", "").replace(".", "").isalnum():
            raise ValueError("invalid stream identifier")
        with self.view(database, "SELECT `Data` FROM `_Streams` WHERE `Name`='" + name + "'") as view:
            self.check(self.MsiViewExecute(view, 0), "read stream")
            record = ctypes.c_uint()
            self.check(self.MsiViewFetch(view, ctypes.byref(record)), "fetch stream")
            try:
                result = bytearray()
                while True:
                    size = ctypes.c_uint(65536)
                    buffer = ctypes.create_string_buffer(size.value)
                    self.check(self.MsiRecordReadStream(record.value, 1, buffer, ctypes.byref(size)), "stream data")
                    result.extend(buffer.raw[:size.value])
                    if size.value < 65536:
                        return bytes(result)
            finally:
                self.MsiCloseHandle(record.value)

    def summary(self, database, values):
        handle = ctypes.c_uint()
        self.check(self.MsiGetSummaryInformationW(database, None, 20, ctypes.byref(handle)), "summary")
        try:
            for property_id, value in values.items():
                kind = 2 if property_id == 1 else 3 if isinstance(value, int) else 30
                self.check(self.MsiSummaryInfoSetPropertyW(handle.value, property_id, kind,
                           value if isinstance(value, int) else 0, None,
                           None if isinstance(value, int) else value), "set summary")
            self.check(self.MsiSummaryInfoPersist(handle.value), "persist summary")
        finally:
            self.MsiCloseHandle(handle.value)
