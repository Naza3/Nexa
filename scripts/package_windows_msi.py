#!/usr/bin/env python3
"""Wrap the exact verified portable payload in one per-user MSI and Setup.exe.

Only Windows inbox MSI/MakeCab and the existing selected MSVC/SDK are used.
No downloader, signing key, external installer framework, or application rebuild.
"""
from __future__ import annotations
import argparse
import ctypes
import copy
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile
import uuid

import package_desktop_windows as desktop
import package_windows as base
from release_version import repository_version, validate_version
from windows_msi_api import Msi

ROOT = base.ROOT
AUTHORING = ROOT / "packaging/windows-installer"
# Published product-family identity: never regenerate when releasing a new version.
UPGRADE_CODE = "{85615F8B-FD70-53D9-86ED-4164379CBC40}"
NAMESPACE = uuid.UUID(UPGRADE_CODE.strip("{}"))
INSTALL_RELATIVE = "Programs/Nexa"
SETUP_RESOURCE = 101
MAX_FILES = 4096


def guid(name):
    return "{" + str(uuid.uuid5(NAMESPACE, name)).upper() + "}"


def product_code(version):
    return guid("product:per-user:x64:" + validate_version(version))


def identifier(prefix, name):
    return prefix + hashlib.sha256(name.casefold().encode("ascii")).hexdigest()[:32]


def checked_relative(name):
    base.relative(name)
    # The closed release payload has ASCII names. Fail closed rather than silently
    # round-trip Unicode file names through a CAB directive/codepage conversion.
    if not name.isascii() or len(name) > 180 or re.search(r'[\x00-\x1f\x7f<>"|?*]', name):
        base.fail("unsupported/unsafe MSI payload path")
    for part in name.split("/"):
        if re.fullmatch(r"(?i)(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\..*)?", part):
            base.fail("Windows device name in MSI payload")
        if "~" in part or len(part) > 120:
            base.fail("ambiguous/overlong MSI payload name")
    return name


def validate_payload(payload, version, *, fixture=False):
    payload = base.resolve_checked_path(payload)
    if not payload.is_dir():
        base.fail("portable payload directory missing")
    manifest = base.strict_json(base.regular(payload / "manifest.json").read_bytes())
    desktop.verify(payload, manifest)
    files = base.entries(payload)
    if not 1 <= len(files) <= MAX_FILES:
        base.fail("MSI payload file count exceeds bounded inventory")
    for item in files:
        checked_relative(item["path"])
        if not 0 <= item["size_bytes"] < 2**31:
            base.fail("MSI payload file size exceeds supported range")
    if not re.fullmatch(r"[a-f0-9]{40}", manifest.get("project_commit", "")) or manifest.get("project_dirty") is not False:
        base.fail("MSI requires a clean exact source identity")
    if not fixture and (version != repository_version(ROOT) or manifest.get("package_version") != version):
        base.fail("MSI/repository/portable versions differ")
    if manifest.get("platform") != "windows-x64" or manifest.get("configuration") != "Release":
        base.fail("MSI requires the Windows x64 Release payload")
    if not (payload / "SHA256SUMS").is_file():
        base.fail("portable checksum file missing")
    return payload, manifest, files


# MSI column definitions, in exact row order. No user input is used in SQL schema.
SCHEMA = {
    "Property": ("Property:s72! Value:l0!", "Property"),
    "Directory": ("Directory:s72! Directory_Parent:s72 DefaultDir:l255!", "Directory"),
    "Component": ("Component:s72! ComponentId:s38 Directory_:s72! Attributes:i2! Condition:s255 KeyPath:s72", "Component"),
    "Feature": ("Feature:s38! Feature_Parent:s38 Title:l64 Description:l255 Display:i2 Level:i2! Directory_:s72 Attributes:i2!", "Feature"),
    "FeatureComponents": ("Feature_:s38! Component_:s72!", "Feature_ Component_"),
    "File": ("File:s72! Component_:s72! FileName:l255! FileSize:i4! Version:s72 Language:s20 Attributes:i2 Sequence:i4!", "File"),
    "Media": ("DiskId:i2! LastSequence:i4! DiskPrompt:l64 Cabinet:s255 VolumeLabel:s32 Source:s72", "DiskId"),
    "Registry": ("Registry:s72! Root:i2! Key:l255! Name:l255 Value:l0 Component_:s72!", "Registry"),
    "Shortcut": ("Shortcut:s72! Directory_:s72! Name:l128! Component_:s72! Target:s72! Arguments:s255 Description:l255 Hotkey:i2 Icon_:s72 IconIndex:i2 ShowCmd:i2 WkDir:s72", "Shortcut"),
    "RemoveFile": ("FileKey:s72! Component_:s72! FileName:l255 DirProperty:s72! InstallMode:i2!", "FileKey"),
    "Upgrade": ("UpgradeCode:s38! VersionMin:s20 VersionMax:s20 Language:s255 Attributes:i4! Remove:s255 ActionProperty:s72!", "UpgradeCode VersionMin VersionMax Language Attributes"),
    "LaunchCondition": ("Condition:s255! Description:l255!", "Condition"),
    "InstallExecuteSequence": ("Action:s72! Condition:s255 Sequence:i2", "Action"),
    "InstallUISequence": ("Action:s72! Condition:s255 Sequence:i2", "Action"),
    "CustomAction": ("Action:s72! Type:i2! Source:s72 Target:l0", "Action"),
    "Binary": ("Name:s72! Data:v0!", "Name"),
    "NexaPayload": ("Path:s180!", "Path"),
}


def columns(table):
    return [field.split(":") for field in SCHEMA[table][0].split()]


def create_sql(table):
    parts = []
    for name, kind in columns(table):
        required = kind.endswith("!")
        kind = kind.rstrip("!")
        type_name = ("SHORT" if kind == "i2" else "LONG" if kind == "i4" else "OBJECT" if kind[0] == "v"
                     else "CHAR(" + kind[1:] + ")")
        parts.append("`" + name + "` " + type_name + (" NOT NULL" if required else "") + (" LOCALIZABLE" if kind[0] == "l" else ""))
    return "CREATE TABLE `" + table + "` (" + ", ".join(parts) + " PRIMARY KEY " + ", ".join("`" + key + "`" for key in SCHEMA[table][1].split()) + ")"


def long_name(path, prefix):
    name = PurePosixPath(path).name
    # A unique explicit 8.3 alias avoids accidental collisions in MSI authoring.
    short = prefix + hashlib.sha256(path.encode("ascii")).hexdigest()[:7].upper()
    return short + "|" + name


def author_tables(files, version, *, rollback_fixture=False):
    validate_version(version)
    if len({item["path"].casefold() for item in files}) != len(files):
        base.fail("duplicate MSI payload path")
    tables = {key: [] for key in SCHEMA}
    props = {
        "ProductCode": product_code(version), "UpgradeCode": UPGRADE_CODE,
        "ProductName": "Nexa", "ProductVersion": version, "ProductLanguage": "1033",
        "Manufacturer": "Nexa", "INSTALLLEVEL": "1", "ARPNOMODIFY": "1", "LIMITUI": "1",
        "ARPHELPLINK": "https://github.com/Naza3/Nexa", "ARPCONTACT": "https://github.com/Naza3/Nexa/issues",
        "SecureCustomProperties": "NEXA_OLDER;NEXA_NEWER", "MSIRESTARTMANAGERCONTROL": "Disable",
        "REBOOT": "ReallySuppress", "MSIFASTINSTALL": "0",
    }
    tables["Property"] = [list(item) for item in sorted(props.items())]
    tables["Directory"] = [
        ["TARGETDIR", None, "SourceDir"], ["LocalAppDataFolder", "TARGETDIR", "."],
        ["NexaProgramsFolder", "LocalAppDataFolder", "Programs"], ["INSTALLFOLDER", "NexaProgramsFolder", "Nexa"],
        ["ProgramMenuFolder", "TARGETDIR", "."], ["NexaMenuFolder", "ProgramMenuFolder", "Nexa"],
    ]
    directories = {"": "INSTALLFOLDER"}
    for item in sorted(files, key=lambda item: item["path"]):
        path = checked_relative(item["path"])
        parent = ""
        for part in PurePosixPath(path).parts[:-1]:
            child = parent + "/" + part if parent else part
            if child not in directories:
                key = identifier("D", child)
                tables["Directory"].append([key, directories[parent], long_name(child, "D")])
                directories[child] = key
            parent = child
        file_id, component = identifier("F", path), identifier("C", path)
        registry = identifier("R", path)
        tables["Component"].append([component, guid("component:per-user:x64:" + path.casefold()), directories[parent], 260, None, registry])
        tables["Registry"].append([registry, 1, "Software\\Nexa\\Installer\\Components", component, "1", component])
        tables["FeatureComponents"].append(["Nexa", component])
        tables["File"].append([file_id, component, long_name(path, "F"), item["size_bytes"], None, None, 16384, len(tables["File"]) + 1])
        tables["NexaPayload"].append([path.replace("/", "\\")])
    meta = "NexaIntegration"
    tables["Component"].append([meta, guid("component:per-user:x64:integration"), "INSTALLFOLDER", 260, None, "NexaInstalledVersion"])
    tables["Registry"].append(["NexaInstalledVersion", 1, "Software\\Nexa\\Installer", "Version", version, meta])
    tables["FeatureComponents"].append(["Nexa", meta])
    tables["Feature"] = [["Nexa", None, "Nexa", "Windows x64 local CPU runtime and desktop manager", 1, 1, "INSTALLFOLDER", 0]]
    tables["Shortcut"] = [["NexaShortcut", "NexaMenuFolder", "Nexa", meta, "[INSTALLFOLDER]nexa-desktop.exe", None,
                            "Nexa desktop manager", None, None, None, 1, "INSTALLFOLDER"]]
    # Empty-directory removal only. No wildcard, recursive removal, service stop,
    # model/config/key path, RemoveFolderEx, or persistent access changes.
    tables["RemoveFile"] = [[identifier("E", path or "root"), meta, None, directory, 2]
                            for path, directory in directories.items()]
    tables["RemoveFile"].append(["NexaRemoveMenu", meta, None, "NexaMenuFolder", 2])
    tables["Media"] = [[1, len(files), None, "#nexa.cab", None, None]]
    tables["Upgrade"] = [[UPGRADE_CODE, None, version, None, 0, None, "NEXA_OLDER"],
                         [UPGRADE_CODE, version, None, None, 2, None, "NEXA_NEWER"]]
    tables["LaunchCondition"] = [
        ["VersionNT64", "Nexa requires 64-bit Windows 10 or later."],
        ["NOT ALLUSERS", "Nexa supports current-user installation only. Do not set ALLUSERS."],
        ["Installed OR NOT NEXA_NEWER", "A newer Nexa version is installed. Downgrades are blocked to protect your data."],
    ]
    tables["CustomAction"] = [["NexaOsGuard", 2, "NexaOsGuardBinary", None], ["NexaGuard", 1, "NexaGuardBinary", "NexaGuard"]]
    tables["InstallExecuteSequence"] = [[action, condition, sequence] for action, condition, sequence in (
        ("FindRelatedProducts", None, 25), ("LaunchConditions", None, 100),
        ("CostInitialize", None, 800), ("FileCost", None, 900), ("CostFinalize", None, 1000),
        ("NexaOsGuard", None, 1050), ("NexaGuard", None, 1100), ("InstallValidate", None, 1400),
        ("InstallInitialize", None, 1500), ("RemoveExistingProducts", None, 1510),
        ("ProcessComponents", None, 1600), ("UnpublishFeatures", None, 1800),
        ("RemoveRegistryValues", None, 2600), ("RemoveShortcuts", None, 3200),
        ("RemoveFiles", None, 3500), ("RemoveFolders", None, 3600), ("CreateFolders", None, 3700),
        ("InstallFiles", None, 4000), ("CreateShortcuts", None, 4500),
        ("WriteRegistryValues", None, 5000), ("RegisterUser", None, 6000),
        ("RegisterProduct", None, 6100), ("PublishFeatures", None, 6300),
        ("PublishProduct", None, 6400), ("InstallFinalize", None, 6600))]
    tables["InstallUISequence"] = [[action, None, sequence] for action, sequence in (
        ("FindRelatedProducts", 25), ("LaunchConditions", 100), ("CostInitialize", 800),
        ("FileCost", 900), ("CostFinalize", 1000), ("NexaOsGuard", 1050), ("NexaGuard", 1100), ("ExecuteAction", 1300))]
    if rollback_fixture:
        # This table exists ONLY in a private CI fixture. It is not a property
        # backdoor in shipped MSI, and there is no CLI switch to generate it.
        tables["CustomAction"].append(["NexaTestRollback", 19, None, "Nexa private rollback fixture"])
        tables["InstallExecuteSequence"].append(["NexaTestRollback", None, 6500])
    return tables


def production_tables(files, version, guard, os_guard, *, rollback_fixture=False):
    tables = author_tables(files, version, rollback_fixture=rollback_fixture)
    tables["Binary"] = [["NexaGuardBinary", guard], ["NexaOsGuardBinary", os_guard]]
    return tables


PREFLIGHT_ACTIONS = ("LaunchConditions", "CostInitialize", "FileCost", "CostFinalize", "NexaLegacyOsProbe", "NexaOsGuard", "NexaGuard")


def preflight_tables(tables, legacy_probe):
    """Private msiexec context test: exact guards, no installation transaction."""
    result = copy.deepcopy(tables)
    for row in result["Property"]:
        if row[0] in ("ProductCode", "UpgradeCode"):
            row[1] = "{" + str(uuid.uuid4()).upper() + "}"
        elif row[0] == "ProductName":
            row[1] = "Nexa guard preflight (not installed)"
    for row in result["Component"]:
        row[1] = "{" + str(uuid.uuid4()).upper() + "}"
    result["Upgrade"] = []
    result["InstallUISequence"] = []
    result["Binary"].append(["NexaLegacyProbeBinary", legacy_probe])
    result["CustomAction"].append(["NexaLegacyOsProbe", 1, "NexaLegacyProbeBinary", "NexaLegacyOsProbe"])
    result["InstallExecuteSequence"] = [row for row in result["InstallExecuteSequence"] if row[0] in PREFLIGHT_ACTIONS]
    result["InstallExecuteSequence"].append(["NexaLegacyOsProbe", None, 1025])
    result["InstallExecuteSequence"].sort(key=lambda row: row[2])
    if tuple(row[0] for row in result["InstallExecuteSequence"]) != PREFLIGHT_ACTIONS:
        base.fail("private preflight action order changed")
    return result


def validate_tables(tables):
    for table, rows in tables.items():
        schema = columns(table)
        keys = [next(i for i, col in enumerate(schema) if col[0] == key) for key in SCHEMA[table][1].split()]
        seen = set()
        for row in rows:
            if len(row) != len(schema):
                base.fail("MSI table field count mismatch")
            key = tuple(row[index] for index in keys)
            if key in seen:
                base.fail("duplicate MSI table primary key")
            seen.add(key)
            for (_, kind), value in zip(schema, row):
                if value is None:
                    if kind.endswith("!"):
                        base.fail("null required MSI table field")
                    continue
                limit = int(kind[1:].rstrip("!"))
                if kind[0] in "sl" and (not isinstance(value, str) or (limit and len(value) > limit)):
                    base.fail("MSI string field exceeds schema")
                if kind[0] == "i" and (type(value) is not int or not -(2**(limit*8-1))+1 <= value < 2**(limit*8-1)):
                    base.fail("MSI integer field exceeds schema")
    dirs = {row[0] for row in tables["Directory"]}
    components = {row[0] for row in tables["Component"]}
    features = {row[0] for row in tables["Feature"]}
    registry = {row[0] for row in tables["Registry"]}
    if any(row[1] and row[1] not in dirs for row in tables["Directory"]):
        base.fail("MSI directory parent missing")
    if any(row[2] not in dirs or row[5] not in registry for row in tables["Component"]):
        base.fail("MSI component directory/keypath missing")
    if any(row[1] not in components for row in tables["File"]):
        base.fail("MSI file component missing")
    if any(row[0] not in features or row[1] not in components for row in tables["FeatureComponents"]):
        base.fail("MSI feature component missing")
    if any(row[1] not in components or row[3] not in dirs or row[2] is not None for row in tables["RemoveFile"]):
        base.fail("MSI unsafe directory removal")
    if len({row[1] for row in tables["Component"]}) != len(components):
        base.fail("duplicate MSI component GUID")


def compile_native(work, msi=None, *, path_test=False, os_check=False, legacy_probe=False):
    if sum((msi is not None, path_test, os_check, legacy_probe)) > 1:
        base.fail("installer helper build roles are mutually exclusive")
    for name in ("CL", "_CL_", "LINK", "_LINK_", "CFLAGS", "CXXFLAGS"):
        if os.environ.get(name):
            base.fail("installer refuses implicit " + name)
    selected, vs, env, _, _ = base.selected_visual_studio()
    compiler = base.selected_msvc_tool(vs, env, "cl.exe")
    dumpbin = base.selected_msvc_tool(vs, env, "dumpbin.exe")
    common = [compiler, "/nologo", "/W4", "/WX", "/O1", "/GS-", "/Zl", "/DUNICODE", "/D_UNICODE", "/D_WIN32_WINNT=0x0A00", "/I" + str(AUTHORING)]
    if msi is None and not os_check:
        output = work / ("nexa-installer-legacy-os-probe.dll" if legacy_probe else "nexa-installer-path-test.dll" if path_test else "nexa-installer-guard.dll")
        source = AUTHORING / ("legacy_os_probe.c" if legacy_probe else "path_test.c" if path_test else "guard.c")
        base.command([*common, "/LD", source, "/Fo" + str(work / "guard.obj"),
                      "/link", "/NODEFAULTLIB", "/NOENTRY", "/MACHINE:X64", "/DYNAMICBASE", "/NXCOMPAT",
                      "/OUT:" + str(output), "/IMPLIB:" + str(work / "guard.lib"), "kernel32.lib", "user32.lib", "msi.lib", "shell32.lib", "ole32.lib", "uuid.lib", "version.lib"], env, cwd=work)
    else:
        output = work / ("nexa-os-check.exe" if os_check else "nexa-setup.exe")
        if not os_check:
            (work / "setup_identity.h").write_text('#define NEXA_MSI_SHA256 "' + base.digest(msi) + '"\n', encoding="ascii")
        rc = Path(env["WINDOWSSDKDIR"]) / "bin" / env["WINDOWSSDKVERSION"].rstrip("\\/") / "x64/rc.exe"
        base.regular(rc)
        resource = work / "setup.rc"
        # RC paths are generated local paths and escaped, never arbitrary directives.
        embedded = "" if os_check else '101 RCDATA "' + str(msi.resolve()).replace("\\", "\\\\").replace('"', '\\"') + '"\n'
        resource.write_text('#include <windows.h>\n' + embedded + '1 RT_MANIFEST "' + str(AUTHORING / "setup.manifest").replace("\\", "\\\\") + '"\n', encoding="utf-8")
        base.command([rc, "/nologo", "/fo", work / "setup.res", resource], env, cwd=work)
        base.command([*common, "/I" + str(work), AUTHORING / ("os_check.c" if os_check else "setup.c"), work / "setup.res", "/Fo" + str(work / "setup.obj"),
                      "/link", "/NODEFAULTLIB", "/ENTRY:" + ("OsCheckEntry" if os_check else "SetupEntry"), "/SUBSYSTEM:WINDOWS", "/MACHINE:X64", "/DYNAMICBASE", "/NXCOMPAT",
                      "/OUT:" + str(output), "kernel32.lib", "user32.lib", "shell32.lib", "ole32.lib", "bcrypt.lib", "comctl32.lib", "uuid.lib", "advapi32.lib", "gdi32.lib"], env, cwd=work)
    base.pe_machine(output)
    imports = base.parse_dependents(base.command([dumpbin, "/nologo", "/dependents", output], env))
    allowed = {"kernel32.dll", "user32.dll", "msi.dll", "shell32.dll", "ole32.dll", "bcrypt.dll", "comctl32.dll", "advapi32.dll", "gdi32.dll", "version.dll"}
    if not set(imports) <= allowed:
        base.fail("installer has non-inbox/CRT dependencies: " + str(imports))
    return output, {"visual_studio": selected["installationVersion"], "msvc": env["VCTOOLSVERSION"], "windows_sdk": env["WINDOWSSDKVERSION"].rstrip("\\/"), "imports": imports}


def check_native_paths(work):
    """Exercise the real guard helper without installing a test product."""
    dll, _ = compile_native(work, path_test=True)
    helper = ctypes.WinDLL(str(dll), use_last_error=True)
    check = helper.NexaTestSameTarget
    check.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
    check.restype = ctypes.c_int
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetShortPathNameW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint]
    kernel.GetShortPathNameW.restype = ctypes.c_uint
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    root = work / "Guard path with spaces"
    root.mkdir()
    alias = ctypes.create_unicode_buffer(32768)
    size = kernel.GetShortPathNameW(str(root), alias, len(alias))
    if not 0 < size < len(alias): base.fail("native guard short-path fixture unavailable")
    external, junction = work / "guard-external", work / "guard-junction"
    external.mkdir()
    results = {}
    try:
        results["existing_alias"] = bool(check(str(root), alias.value))
        results["missing_tail_alias"] = bool(check(str(root / "missing/Programs/Nexa"), str(Path(alias.value) / "missing/Programs/Nexa")))
        results["dot_tail_rejected"] = not check(str(root / "missing/../Nexa"), str(root / "Nexa"))
        results["wrong_target_rejected"] = not check(str(root / "Nexa"), str(external / "Nexa"))
        cmd = base.regular(Path(os.environ["SYSTEMROOT"]) / "System32/cmd.exe")
        base.command([cmd, "/d", "/c", "mklink", "/J", junction, external], cwd=work)
        results["junction_rejected"] = not check(str(junction / "missing/Nexa"), str(external / "missing/Nexa"))
        if not all(results.values()): base.fail("native guard path regression failed: " + json.dumps(results, sort_keys=True))
    finally:
        if junction.exists(): os.rmdir(junction)
        kernel.FreeLibrary(helper._handle)
    return {"checks": results, "short_alias_available": str(root).casefold() != alias.value.casefold()}


def make_cabinet(payload, files, work):
    cab_input = work / "cab-input"
    cab_input.mkdir()
    lines = [".OPTION EXPLICIT", ".Set CabinetNameTemplate=nexa.cab", ".Set DiskDirectoryTemplate=.",
             ".Set Cabinet=on", ".Set Compress=on", ".Set CompressionType=LZX", ".Set CompressionMemory=21",
             ".Set MaxDiskSize=0", ".Set CabinetFileCountThreshold=0", ".Set FolderFileCountThreshold=0"]
    for item in sorted(files, key=lambda item: item["path"]):
        name = identifier("F", item["path"])
        shutil.copyfile(base.regular(payload / item["path"]), cab_input / name)
        if base.digest(cab_input / name) != item["sha256"]:
            base.fail("payload changed during cabinet staging")
        lines.append('"cab-input\\' + name + '" ' + name)
    ddf = work / "payload.ddf"
    ddf.write_text("\r\n".join(lines) + "\r\n", encoding="ascii")
    makecab = base.regular(Path(os.environ["SYSTEMROOT"]) / "System32/makecab.exe")
    base.command([makecab, "/F", ddf], cwd=work)
    return base.regular(work / "nexa.cab")


def write_msi(files, version, cab, guard, output, *, rollback_fixture=False, preflight_probe=None):
    os_guard = base.regular(guard.parent / "nexa-os-check.exe")
    tables = production_tables(files, version, guard, os_guard, rollback_fixture=rollback_fixture)
    if preflight_probe is not None:
        tables = preflight_tables(tables, preflight_probe)
    validate_tables(tables)
    api = Msi()
    package_code = "{" + str(uuid.uuid4()).upper() + "}"
    with api.database(output, 3) as database:
        for table, rows in tables.items():
            api.execute(database, create_sql(table))
            names = ["`" + name + "`" for name, _ in columns(table)]
            sql = "INSERT INTO `" + table + "` (" + ",".join(names) + ") VALUES (" + ",".join("?" for _ in names) + ")"
            for row in rows:
                api.execute(database, sql, row)
        api.execute(database, "INSERT INTO `_Streams` (`Name`,`Data`) VALUES (?,?)", ["nexa.cab", cab])
        api.summary(database, {1: 1252, 2: "Nexa Setup", 3: "Nexa per-user Windows x64 desktop", 4: "Nexa",
                              5: "Installer", 7: "x64;1033", 9: package_code, 14: 500, 15: 10, 18: "Nexa native MSI packager", 19: 2})
        api.check(api.MsiDatabaseCommit(database), "commit")
    api.check(api.MsiVerifyPackageW(str(output)), "verify package")
    with api.database(output) as database:
        if api.summary_string(database, 9) != package_code:
            base.fail("MSI PackageCode readback mismatch")
        for table, expected in tables.items():
            if table == "Binary":
                continue
            actual = api.rows(database, "SELECT * FROM `" + table + "`")
            normalized = [["" if value is None else str(value) for value in row] for row in expected]
            if sorted(actual) != sorted(normalized):
                base.fail("MSI table readback mismatch: " + table)
        for name, binary in tables["Binary"]:
            if hashlib.sha256(api.stream(database, "Binary." + name)).hexdigest() != base.digest(binary):
                base.fail("MSI embedded guard differs from compiled helper")
        if hashlib.sha256(api.stream(database, "nexa.cab")).hexdigest() != base.digest(cab):
            base.fail("MSI embedded cabinet differs")
    return package_code


def check_msi_guard_context(work, guard, os_guard):
    """Use real msiexec before the long build, with only read-only guard actions."""
    from windows_installer_diagnostics import Trace, summarize_log
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_OS") != "Windows":
        base.fail("MSI guard preflight requires a disposable GitHub Windows runner")
    root = Path(os.environ["LOCALAPPDATA"]) / INSTALL_RELATIVE
    data = Path(os.environ["LOCALAPPDATA"]) / "Nexa"
    if root.exists() or data.exists():
        base.fail("guard preflight refuses existing Nexa installation or user data")
    commit = base.command(["git", "rev-parse", "HEAD"])
    trace = Trace(ROOT / "dist/windows-msi-diagnostics.json", commit, repository_version(ROOT))
    trace.event("installer_guard_preflight", "start", expected=(0,))
    probe = work / "guard-context.msi"
    log = work / "guard-context.log"
    try:
        direct = subprocess.run([str(os_guard)], timeout=30, check=False)
        if direct.returncode != 0:
            trace.event("installer_guard_preflight", "error", exit_code=direct.returncode, expected=(0,))
            base.fail("manifest-bearing OS helper rejected this host: " + str(direct.returncode))
        legacy, _ = compile_native(work, legacy_probe=True)
        payload = work / "probe-payload"
        payload.mkdir()
        (payload / "README.txt").write_text("Private guard preflight. Never installed or released.\n", encoding="ascii")
        files = base.entries(payload)
        cab = make_cabinet(payload, files, work)
        write_msi(files, repository_version(ROOT), cab, guard, probe, preflight_probe=legacy)
        api = Msi()
        with api.database(probe) as database:
            identity = dict(api.rows(database, "SELECT `Property`,`Value` FROM `Property`"))["ProductCode"]
        before = api.MsiQueryProductStateW(identity)
        if before != -1: base.fail("private guard probe product unexpectedly registered")
        executable = base.regular(Path(os.environ["SYSTEMROOT"]) / "System32/msiexec.exe")
        process = subprocess.Popen([str(executable), "/i", str(probe), "/qn", "/norestart", "/l*v", str(log), "REBOOT=ReallySuppress"], stdin=subprocess.DEVNULL)
        try:
            code = process.wait(timeout=60)
        except subprocess.TimeoutExpired as error:
            trace.event("installer_guard_preflight", "timeout", expected=(0,), log=log, pid=process.pid)
            raise ValueError("read-only MSI guard preflight exceeded 60 seconds") from error
        trace.event("installer_guard_preflight", "complete" if code == 0 else "error", exit_code=code, expected=(0,), log=log, pid=process.pid)
        after = api.MsiQueryProductStateW(identity)
        if code != 0 or after != -1 or root.exists() or data.exists():
            base.fail("read-only MSI guard preflight failed or changed product/user state")
        observation = summarize_log(log)
        required = {"NexaLegacyOsProbe", "NexaOsGuard", "NexaGuard"}
        ended = {item["action"] for item in observation["actions"] if item["phase"] == "end" and item["return_code"] == 1}
        if not required <= ended or not observation["legacy_os"]:
            base.fail("real MSI-context guard execution was not observed")
        rejected = {}
        for stage, properties in (
            ("preflight_wrong_scope", ["ALLUSERS=1"]),
            ("preflight_wrong_directory", ["INSTALLFOLDER=" + str(work / "forbidden-target")]),
        ):
            negative_log = work / (stage + ".log")
            trace.event(stage, "start", expected=(1603,))
            negative = subprocess.Popen([str(executable), "/i", str(probe), "/qn", "/norestart", "/l*v", str(negative_log),
                                         "REBOOT=ReallySuppress", *properties], stdin=subprocess.DEVNULL)
            try:
                rejected_code = negative.wait(timeout=60)
            except subprocess.TimeoutExpired as error:
                trace.event(stage, "timeout", expected=(1603,), log=negative_log, pid=negative.pid)
                raise ValueError("read-only MSI rejection probe exceeded 60 seconds") from error
            contexts = api.product_contexts(identity)
            unchanged = api.MsiQueryProductStateW(identity) == -1 and not contexts and not root.exists() and not data.exists() and not (work / "forbidden-target").exists()
            trace.event(stage, "complete" if rejected_code == 1603 and unchanged else "error", exit_code=rejected_code,
                        expected=(1603,), log=negative_log, pid=negative.pid, contexts=contexts, targets_unchanged=unchanged)
            if rejected_code != 1603 or not unchanged:
                base.fail("fresh MSI override was not safely rejected: " + stage)
            rejected[stage] = True
        # A passing preflight is still not a passing installation lifecycle.
        return {"direct_os_check_exit_code": direct.returncode, "msiexec_exit_code": code,
                "private_product_registered": False, "nexa_directories_created": False,
                "production_guards_executed": True, "legacy_os": observation["legacy_os"], "fresh_overrides_rejected": rejected}
    except (ValueError, OSError, subprocess.SubprocessError):
        if trace.document["status"] != "failed":
            trace.event("installer_guard_preflight", "error", expected=(0,), log=log)
        raise


def setup_embedded_msi(setup):
    # Parse PE resources without executing Setup or relying on non-inbox packages.
    if sys.platform != "win32":
        base.fail("native Windows resource verification required")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    module = kernel.LoadLibraryExW(str(setup), None, 2 | 32)
    if not module:
        base.fail("Setup resource image could not be opened")
    try:
        resource = kernel.FindResourceW(module, ctypes.c_void_p(SETUP_RESOURCE), ctypes.c_void_p(10))
        size = kernel.SizeofResource(module, resource) if resource else 0
        handle = kernel.LoadResource(module, resource) if size else None
        pointer = kernel.LockResource(handle) if handle else None
        if not pointer or not size:
            base.fail("Setup embedded MSI is missing")
        return ctypes.string_at(pointer, size)
    finally:
        kernel.FreeLibrary(module)


def build(payload, version, output, setup_output, report):
    version = validate_version(version)
    payload, manifest, files = validate_payload(payload, version)
    destinations = [base.resolve_checked_path(Path(path)) for path in (output, setup_output, report)]
    if len(set(destinations)) != 3 or any(path.exists() or path.is_relative_to(payload) for path in destinations):
        base.fail("installer outputs must be distinct new files outside portable payload")
    output, setup_output, report = destinations
    for path in destinations:
        path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nexa-msi-", dir=output.parent) as temporary:
        work = Path(temporary)
        guard, guard_tools = compile_native(work)
        _, os_tools = compile_native(work, os_check=True)
        cab = make_cabinet(payload, files, work)
        staged_msi = work / "nexa.msi"
        package_code = write_msi(files, version, cab, guard, staged_msi)
        setup, setup_tools = compile_native(work, staged_msi)
        if setup_embedded_msi(setup) != staged_msi.read_bytes():
            base.fail("Setup does not embed the exact final MSI")
        if base.entries(payload) != files:
            base.fail("portable payload changed during installer packaging")
        result = {"schema_version": 1, "status": "pass", "version": version,
                  "project_commit": manifest["project_commit"], "payload_manifest_sha256": base.digest(payload / "manifest.json"),
                  "msi_sha256": base.digest(staged_msi), "setup_sha256": base.digest(setup),
                  "product_code": product_code(version), "upgrade_code": UPGRADE_CODE, "package_code": package_code,
                  "unsigned": True, "scope": "per-user", "toolchain": {"guard": guard_tools, "os_check": os_tools, "setup": setup_tools},
                  "checks": {"payload_verified": True, "msi_tables_verified": True, "setup_embeds_exact_msi": True}}
        shutil.copyfile(staged_msi, output)
        shutil.copyfile(setup, setup_output)
        base.write_json(report, result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-toolchain", action="store_true", help="compile helpers and run private read-only MSI guard preflight; does not install application payload")
    parser.add_argument("--payload", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--setup-output", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("MSI and Setup creation require a native Windows x64 runner")
    if not args.check_toolchain and any(value is None for value in (args.payload, args.version, args.output, args.setup_output)):
        parser.error("--payload, --version, --output and --setup-output are required for packaging")
    try:
        if args.check_toolchain:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix="nexa-installer-compile-", dir=args.report.parent) as temporary:
                work = Path(temporary).resolve()
                guard, guard_tools = compile_native(work)
                os_guard, os_tools = compile_native(work, os_check=True)
                guard_path_checks = check_native_paths(work)
                guard_context = check_msi_guard_context(work, guard, os_guard)
                fixture = work / "compile-only-resource.bin"
                fixture.write_bytes(b"Nexa compile-only resource. This is not an MSI package.")
                _, setup_tools = compile_native(work, fixture)
                base.write_json(args.report, {"schema_version": 1, "status": "compile-pass", "guard": guard_tools, "setup": setup_tools, "os_check": os_tools, "guard_path_checks": guard_path_checks, "msi_guard_context": guard_context, "installation_tested": False})
            print("Installer helpers and real MSI-context preflight passed; no application installation or payload validation was performed")
            return 0
        result = build(args.payload, args.version, args.output, args.setup_output, args.report)
    except (ValueError, OSError, KeyError) as error:
        print(str(error), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
