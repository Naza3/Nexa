#!/usr/bin/env python3
"""Real Tauri MSI/NSIS acceptance, restricted to disposable GitHub Windows runners.

This program deliberately installs and removes its own private fixtures. It
refuses existing Nexa data, registrations and program directories. Never run it
on an end user's machine. Private changed-payload/future/failure packages are
not release assets and do not pass the production payload-verification path.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

import package_tauri_windows as pack
import package_windows_msi as legacy
import test_windows_msi_lifecycle as common
from windows_installer_diagnostics import Trace
from windows_msi_api import Msi


CHECKS = (
    "msi_install", "msi_repair", "msi_same_version_upgrade", "msi_rollback",
    "msi_upgrade", "downgrade_rejected", "msi_uninstall", "legacy_msi_migration",
    "nsis_install", "nsis_repeat_install", "nsis_same_version_upgrade", "nsis_uninstall",
    "running_process_blocked", "cross_format_blocked", "user_data_preserved", "per_user_scope",
    "autostart_lifecycle",
)
STAGES = frozenset("""
    preflight fixture_build msi_legacy_install msi_legacy_migrate msi_repair
    runtime_init runtime_ready runtime_stop msi_busy_upgrade msi_busy_uninstall
    msi_cross_format nsis_cross_format msi_same_rollback msi_same_upgrade
    msi_upgrade msi_downgrade msi_uninstall msi_install nsis_install
    nsis_repeat_install nsis_same_upgrade nsis_busy_install nsis_busy_uninstall
    nsis_uninstall nsis_upgrade nsis_downgrade complete
    msi_legacy_release_install msi_legacy_release_migrate msi_legacy_release_uninstall
""".split())
RELEASED_LEGACY_VERSION = "0.2.3"
NSIS_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nexa"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
require = common.require


def msi_identity(api, msi):
    with api.database(msi) as database:
        properties = dict(api.rows(database, "SELECT `Property`,`Value` FROM `Property`"))
        package = api.summary_string(database, 9)
    for value in (properties.get("ProductCode", ""), properties.get("UpgradeCode", ""), package):
        try:
            uuid.UUID(value.strip("{}"))
        except (AttributeError, ValueError) as error:
            raise ValueError("installer product identity is invalid") from error
    require(properties["UpgradeCode"].upper() == legacy.UPGRADE_CODE,
            "Tauri MSI does not retain the existing Nexa product family")
    version = legacy.validate_version(properties.get("ProductVersion", ""))
    return properties["ProductCode"], version, package


def related_products(api):
    """Enumerate this user's/MSI-machine Nexa family, without reading user SIDs."""
    api.bind("MsiEnumRelatedProductsW", [ctypes.c_wchar_p, ctypes.c_uint,
                                       ctypes.c_uint, ctypes.c_wchar_p])
    products = []
    for index in range(32):
        product = ctypes.create_unicode_buffer(39)
        code = api.MsiEnumRelatedProductsW(legacy.UPGRADE_CODE, 0, index, product)
        if code == 259:
            return sorted(products)
        api.check(code, "enumerate related Nexa products")
        products.append(product.value.upper())
    raise ValueError("Nexa related-product enumeration exceeded its bound")


def nsis_registrations():
    import winreg
    result = set()
    for label, hive in (("user", winreg.HKEY_CURRENT_USER), ("machine", winreg.HKEY_LOCAL_MACHINE)):
        for access in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
            try:
                key = winreg.OpenKey(hive, NSIS_KEY, 0, winreg.KEY_READ | access)
            except FileNotFoundError:
                continue
            with key:
                try:
                    uninstall, _ = winreg.QueryValueEx(key, "UninstallString")
                    version, _ = winreg.QueryValueEx(key, "DisplayVersion")
                except FileNotFoundError as error:
                    raise ValueError("Nexa has an incomplete NSIS registration") from error
                # HKCU\Software is shared across WOW64 views on supported Windows.
                # Checking both views must not invent duplicate registrations.
                result.add((label, uninstall, version))
    return sorted(result)


def autostart_value():
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_READ) as key:
            value, kind = winreg.QueryValueEx(key, "Nexa")
    except FileNotFoundError:
        return None
    require(kind == winreg.REG_SZ, "Nexa autostart registry value has an unexpected type")
    return value


def write_test_autostart(value):
    import winreg
    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
        if value is None:
            winreg.DeleteValue(key, "Nexa")
        else:
            winreg.SetValueEx(key, "Nexa", 0, winreg.REG_SZ, value)


def assert_msi(api, product, root, files, sentinels, machine_targets):
    require(api.MsiQueryProductStateW(product) == 5 and api.product_contexts(product) == [2],
            "MSI is not registered solely in the current-user unmanaged context")
    require(related_products(api) == [product.upper()], "MSI replacement left duplicate product registrations")
    require(not nsis_registrations(), "MSI installation unexpectedly registered NSIS")
    common.installed_payload(root, files)
    common.same_sentinels(sentinels)
    require(all(not path.exists() for path in machine_targets), "MSI created machine-wide targets")


def assert_nsis(api, version, root, files, sentinels, machine_targets):
    expected = [("user", '"' + str(root / "uninstall.exe") + '"', version)]
    require(nsis_registrations() == expected, "NSIS registration has the wrong scope, version or uninstall target")
    require(not related_products(api), "NSIS installation left an MSI registration")
    require((root / "uninstall.exe").is_file(), "NSIS uninstaller is missing")
    common.installed_payload(root, files)
    common.same_sentinels(sentinels)
    require(all(not path.exists() for path in machine_targets), "NSIS created machine-wide targets")


def assert_removed(root, files, sentinels):
    require(all(not (root / item["path"]).exists() for item in files), "uninstall left an owned payload file")
    common.same_sentinels(sentinels)


def variant_payload(payload, destination, *, legacy_fixture=False):
    """Private fixtures exercise changed bytes, removed files and added files."""
    shutil.copytree(payload, destination)
    readme = destination / "README.md"
    require(readme.is_file(), "payload has no README for the changed-byte fixture")
    with readme.open("ab") as output:
        output.write(b"\nNexa private lifecycle payload variant.\n")
    if legacy_fixture:
        added = destination / "nexa-private-obsolete.txt"
    else:
        removed = destination / "SHA256SUMS"
        require(removed.is_file(), "payload has no checksum inventory for the removal fixture")
        removed.unlink()
        added = destination / "private-fixture/added.txt"
        added.parent.mkdir()
    added.write_bytes(b"Nexa private installer-owned fixture, never a release asset.\n")
    return legacy.base.entries(destination)


def build_tauri_fixture(payload, files, version, work, *, kinds=("msi", "nsis")):
    work.mkdir()
    helpers = pack.compile_helpers(work / "helpers", files)
    project = pack.prepare_project(payload, files, version, work, helpers)
    return {kind: pack.bundle(project, kind, work, version) for kind in kinds}


def build_released_legacy_fixture(files, version, cabinet, guard, work):
    """Exercise the released legacy identity, not historical release binaries.

    The inventory and helpers remain our private, locally authored fixture.
    Equal/older targets keep the existing same-version migration test only.
    """
    current = tuple(map(int, legacy.validate_version(version).split(".")))
    if current <= tuple(map(int, RELEASED_LEGACY_VERSION.split("."))):
        return None
    destination = work / "legacy-release-0.2.3.msi"
    legacy.write_msi(files, RELEASED_LEGACY_VERSION, cabinet, guard, destination)
    return destination


def add_rollback_fixture(source, destination, api=None):
    """Change only a private copy, with a new PackageCode and checked failure."""
    api = api or Msi()
    require(source != destination and not destination.exists(), "rollback fixture must be a new private MSI")
    shutil.copyfile(source, destination)
    replacement = "{" + str(uuid.uuid4()).upper() + "}"
    with api.database(destination, 1) as database:
        previous = api.summary_string(database, 9)
        require(previous != replacement, "rollback fixture reused its PackageCode")
        actions = dict(api.rows(database, "SELECT `Action`,`Sequence` FROM `InstallExecuteSequence`"))
        # Execute the queued file operations before the deliberate failure so
        # rollback is checked after actual replacement, not just scheduling.
        if "InstallExecute" not in actions:
            api.execute(database, "INSERT INTO `InstallExecuteSequence` (`Action`,`Condition`,`Sequence`) VALUES (?,?,?)",
                        ["InstallExecute", None, 6490])
        else:
            require(int(actions["InstallExecute"]) < 6590, "rollback fixture cannot follow InstallExecute")
        api.execute(database, "INSERT INTO `CustomAction` (`Action`,`Type`,`Source`,`Target`) VALUES (?,?,?,?)",
                    ["NexaTestRollback", 19, None, "Nexa private rollback fixture"])
        api.execute(database, "INSERT INTO `InstallExecuteSequence` (`Action`,`Condition`,`Sequence`) VALUES (?,?,?)",
                    ["NexaTestRollback", "NOT Installed", 6590])
        api.summary(database, {9: replacement})
        api.check(api.MsiDatabaseCommit(database), "commit private rollback fixture")
    with api.database(destination) as database:
        require(api.summary_string(database, 9) == replacement, "rollback PackageCode did not persist")
    return destination


@contextmanager
def live_runtime(root, work):
    runtime = root / "runtime/ai-runtime.exe"
    data = work / "runtime-data"
    common.invoke([runtime, "--data-dir", data, "init"], stage="runtime_init")
    common.trace("runtime_ready", "start")
    process = subprocess.Popen([str(common.short_path(runtime)), "--data-dir", str(data), "serve"],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            require(process.poll() is None, "installed runtime exited before the busy test")
            status = subprocess.run([str(runtime), "--data-dir", str(data), "status"],
                                    capture_output=True, timeout=10)
            if status.returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise ValueError("installed runtime did not become ready")
        common.trace("runtime_ready", "complete")
        yield process
    finally:
        if process.poll() is None:
            common.invoke([runtime, "--data-dir", data, "stop"], stage="runtime_stop")
            require(process.wait(timeout=30) == 0, "runtime did not stop normally")


def uninstall_nsis(root, *, accepted=(0,), stage="nsis_uninstall"):
    # NSIS _?= must be the final, unquoted tail. It keeps execution in the actual
    # uninstaller, so the parent observes its result rather than a launcher PID.
    # Every path here comes from the fixed, verified test install root.
    uninstaller = legacy.base.regular(root / "uninstall.exe")
    command = subprocess.list2cmdline([str(uninstaller), "/S"]) + " _?=" + str(root)
    common.trace(stage, "start", expected=accepted)
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL)
    try:
        code = process.wait(timeout=240)
    except subprocess.TimeoutExpired as error:
        common.trace(stage, "timeout", expected=accepted, pid=process.pid)
        raise ValueError("NSIS uninstaller did not finish within the acceptance deadline") from error
    common.trace(stage, "complete" if code in accepted else "error", exit_code=code,
                 expected=accepted, pid=process.pid)
    require(code in accepted, "NSIS uninstaller returned an unexpected exit code")
    if code == 0:
        # A running _?= uninstaller cannot delete its own executable. Remove only
        # that known test-owned file, after it has exited and removed registration.
        require(not nsis_registrations(), "NSIS uninstaller left its product registration")
        uninstaller.unlink(missing_ok=True)
    return code


def lifecycle(msi, setup, payload, report):
    require(sys.platform == "win32" and os.environ.get("GITHUB_ACTIONS") == "true"
            and os.environ.get("RUNNER_OS") == "Windows",
            "lifecycle acceptance is restricted to disposable GitHub Windows runners")
    api = Msi()
    msi, setup = legacy.base.regular(msi), legacy.base.regular(setup)
    version = legacy.repository_version(legacy.ROOT)
    payload, manifest, files = legacy.validate_payload(payload, version)
    product, msi_version, package_code = msi_identity(api, msi)
    require(msi_version == version, "production MSI version differs from the verified payload")
    root = Path(os.environ["LOCALAPPDATA"]) / "Programs/Nexa"
    data = Path(os.environ["LOCALAPPDATA"]) / "Nexa"
    menu = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Nexa"
    machine_targets = [Path(os.environ["ProgramFiles"]) / "Nexa",
                       Path(os.environ["ProgramFiles(x86)"]) / "Nexa",
                       Path(os.environ["ProgramData"]) / "Microsoft/Windows/Start Menu/Programs/Nexa"]
    require(not root.exists() and not data.exists() and not menu.exists(),
            "refusing existing Nexa program/user data or Start Menu directories")
    require(not related_products(api) and not nsis_registrations(), "refusing existing Nexa installer registrations")
    require(autostart_value() is None, "refusing an existing Nexa autostart value")
    require(all(not path.exists() for path in machine_targets), "refusing existing Nexa machine-wide targets")
    report = legacy.base.resolve_checked_path(Path(report))
    require(not report.exists() and not report.is_relative_to(payload), "lifecycle report must be new and outside the payload")
    report.parent.mkdir(parents=True, exist_ok=True)
    logs = report.parent / "windows-msi-logs"
    logs.mkdir(exist_ok=True)
    checks = dict.fromkeys(CHECKS, False)
    result = {"schema_version": 1, "status": "failed", "version": version,
              "project_commit": manifest["project_commit"],
              "payload_manifest_sha256": legacy.base.digest(payload / "manifest.json"),
              "msi_sha256": legacy.base.digest(msi), "setup_sha256": legacy.base.digest(setup),
              "product_code": product, "upgrade_code": legacy.UPGRADE_CODE,
              "unsigned": True, "packager": "tauri", "checks": checks,
              "windows10_target_hardware": "not_run"}
    common._TRACE = Trace(report.with_name("windows-msi-diagnostics.json"), manifest["project_commit"], version)
    common.trace("preflight", "complete")
    # Retain private fixtures on failure for this disposable runner's diagnosis.
    # Never remove an installation tree or kill an active installer in cleanup.
    work = Path(tempfile.mkdtemp(prefix="nexa-tauri-acceptance-"))
    try:
        common.trace("fixture_build", "start")
        legacy_payload = work / "legacy-payload"
        legacy_files = variant_payload(payload, legacy_payload, legacy_fixture=True)
        legacy_work = work / "legacy"
        legacy_work.mkdir()
        helpers = pack.compile_helpers(legacy_work / "helpers", legacy_files)
        cabinet = legacy.make_cabinet(legacy_payload, legacy_files, legacy_work)
        old_msi = legacy_work / "legacy.msi"
        legacy.write_msi(legacy_files, version, cabinet, helpers["guard"], old_msi)
        old_product, _, _ = msi_identity(api, old_msi)
        require(old_product == legacy.product_code(version) and old_product != product,
                "legacy fixture does not have the original version-only product identity")
        released_legacy = build_released_legacy_fixture(
            legacy_files, version, cabinet, helpers["guard"], legacy_work)
        changed_payload = work / "changed-payload"
        changed_files = variant_payload(payload, changed_payload)
        changed = build_tauri_fixture(changed_payload, changed_files, version, work / "changed")
        changed_product, changed_version, changed_package = msi_identity(api, changed["msi"])
        require(changed_product not in (old_product, product) and changed_version == version
                and changed_package != package_code,
                "same-version Tauri fixture did not receive a distinct product identity")
        broken = add_rollback_fixture(changed["msi"], work / "rollback.msi", api)
        future_version = common.next_version(version)
        future = build_tauri_fixture(payload, files, future_version, work / "future")
        future_product, built_future_version, _ = msi_identity(api, future["msi"])
        require(future_product not in (old_product, product, changed_product)
                and built_future_version == future_version, "future MSI fixture identity is invalid")
        common.trace("fixture_build", "complete")

        if released_legacy is not None:
            released_product, released_version, _ = msi_identity(api, released_legacy)
            require(released_version == RELEASED_LEGACY_VERSION
                    and released_product == legacy.product_code(RELEASED_LEGACY_VERSION),
                    "released legacy fixture does not have the original 0.2.3 product identity")
            common.msi_command(released_legacy, "/i", logs / "legacy-release-install.log")
            assert_msi(api, released_product, root, legacy_files, {}, machine_targets)
            common.msi_command(msi, "/i", logs / "legacy-release-migrate.log")
            assert_msi(api, product, root, files, {}, machine_targets)
            require(api.MsiQueryProductStateW(released_product) == -1
                    and not (root / "nexa-private-obsolete.txt").exists(),
                    "0.2.3 legacy migration left the old product or obsolete owned file")
            common.msi_command(msi, "/x", logs / "legacy-release-uninstall.log")
            assert_removed(root, files, {})
            require(not related_products(api), "0.2.3 migration cleanup retained an MSI registration")
            require(autostart_value() is None, "0.2.3 migration enabled autostart")

        common.msi_command(old_msi, "/i", logs / "legacy-install.log")
        common.installed_payload(root, legacy_files)
        require(api.MsiQueryProductStateW(old_product) == 5, "legacy MSI fixture was not installed")
        require(autostart_value() is None, "legacy fixture automatically enabled autostart")
        enabled_autostart = '"' + str(root / "nexa-desktop.exe") + '"'
        foreign_autostart = '"' + str(work / "unrelated-program.exe") + '"'
        write_test_autostart(enabled_autostart)
        sentinels = {data / "config.toml": b"# CI preservation sentinel, not a runtime config\n",
                     data / "auth.token": b"CI-NOT-A-CREDENTIAL\n",
                     data / "model-library.json": b'{"ci-preserve":true}\n',
                     data / "models/keep.gguf": b"CI non-model preservation sentinel",
                     root / "models/keep.gguf": b"CI installed-folder model sentinel",
                     root / "model/keep.gguf": b"CI legacy model directory sentinel",
                     work / "external/keep.gguf": b"CI external model sentinel"}
        for path, contents in sentinels.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
        common.msi_command(msi, "/i", logs / "legacy-migrate.log")
        assert_msi(api, product, root, files, sentinels, machine_targets)
        require(api.MsiQueryProductStateW(old_product) == -1 and not (root / "nexa-private-obsolete.txt").exists(),
                "legacy MSI migration left the old product or obsolete owned file")
        require(autostart_value() == enabled_autostart, "legacy migration changed autostart")
        checks["legacy_msi_migration"] = True

        (root / "README.md").write_bytes(b"private repair corruption")
        common.msi_command(msi, "/fa", logs / "repair.log")
        assert_msi(api, product, root, files, sentinels, machine_targets)
        checks["msi_repair"] = True
        common.invoke([setup, "/S"], (1638,), stage="nsis_cross_format")
        assert_msi(api, product, root, files, sentinels, machine_targets)
        with live_runtime(root, work / "msi-busy") as process:
            common.msi_command(changed["msi"], "/i", logs / "busy-upgrade.log", accepted=(1603,))
            common.msi_command(msi, "/x", logs / "busy-uninstall.log", accepted=(1603,))
            require(process.poll() is None, "MSI stopped the live runtime")
            assert_msi(api, product, root, files, sentinels, machine_targets)

        common.msi_command(broken, "/i", logs / "same-rollback.log", accepted=(1603,))
        assert_msi(api, product, root, files, sentinels, machine_targets)
        require(api.MsiQueryProductStateW(changed_product) == -1 and not (root / "private-fixture/added.txt").exists(),
                "failed same-version replacement left new registration/files")
        require(autostart_value() == enabled_autostart, "failed MSI upgrade changed autostart")
        checks["msi_rollback"] = True
        common.msi_command(changed["msi"], "/i", logs / "same-upgrade.log")
        assert_msi(api, changed_product, root, changed_files, sentinels, machine_targets)
        require(api.MsiQueryProductStateW(product) == -1 and not (root / "SHA256SUMS").exists(),
                "same-version MSI replacement retained the previous product/owned file")
        require(autostart_value() == enabled_autostart, "same-version MSI upgrade changed autostart")
        checks["msi_same_version_upgrade"] = True
        common.msi_command(future["msi"], "/i", logs / "upgrade.log")
        assert_msi(api, future_product, root, files, sentinels, machine_targets)
        require(not (root / "private-fixture/added.txt").exists(), "higher-version upgrade retained an obsolete file")
        require(autostart_value() == enabled_autostart, "higher-version MSI upgrade changed autostart")
        checks["msi_upgrade"] = True
        common.msi_command(msi, "/i", logs / "downgrade.log", accepted=(1603,))
        assert_msi(api, future_product, root, files, sentinels, machine_targets)
        common.msi_command(future["msi"], "/x", logs / "uninstall.log")
        assert_removed(root, files, sentinels)
        require(not related_products(api), "MSI uninstall retained product registration")
        require(autostart_value() is None, "MSI uninstall retained its owned autostart entry")
        checks["msi_uninstall"] = True
        # Also install the unmodified release MSI without a related product.
        common.msi_command(msi, "/i", logs / "install.log")
        assert_msi(api, product, root, files, sentinels, machine_targets)
        require(autostart_value() is None, "MSI install automatically enabled autostart")
        # Deliberately exceed the guard's bounded read: this is foreign data,
        # not a command owned by Nexa, and must not turn uninstall into failure.
        oversized_foreign_autostart = foreign_autostart + " " + "x" * 40000
        write_test_autostart(oversized_foreign_autostart)
        checks["msi_install"] = True
        common.msi_command(msi, "/x", logs / "uninstall.log")
        assert_removed(root, files, sentinels)
        require(autostart_value() == oversized_foreign_autostart, "MSI uninstall changed oversized unrelated autostart data")
        write_test_autostart(None)

        common.invoke([setup, "/S"], stage="nsis_install")
        assert_nsis(api, version, root, files, sentinels, machine_targets)
        require(autostart_value() is None, "NSIS install automatically enabled autostart")
        write_test_autostart(enabled_autostart)
        checks["nsis_install"] = True
        common.msi_command(msi, "/i", logs / "cross-format.log", accepted=(1603,))
        assert_nsis(api, version, root, files, sentinels, machine_targets)
        checks["cross_format_blocked"] = True
        (root / "README.md").write_bytes(b"private NSIS repeat-install corruption")
        common.invoke([setup, "/S"], stage="nsis_repeat_install")
        assert_nsis(api, version, root, files, sentinels, machine_targets)
        checks["nsis_repeat_install"] = True
        require(autostart_value() == enabled_autostart, "NSIS repeat installation changed autostart")
        with live_runtime(root, work / "nsis-busy") as process:
            common.invoke([changed["nsis"], "/S"], (1603,), stage="nsis_busy_install")
            uninstall_nsis(root, accepted=(1603,), stage="nsis_busy_uninstall")
            require(process.poll() is None, "NSIS stopped the live runtime")
            assert_nsis(api, version, root, files, sentinels, machine_targets)
        checks["running_process_blocked"] = True
        common.invoke([changed["nsis"], "/S"], stage="nsis_same_upgrade")
        assert_nsis(api, version, root, changed_files, sentinels, machine_targets)
        require(not (root / "SHA256SUMS").exists(), "NSIS replacement retained an obsolete owned file")
        require(autostart_value() == enabled_autostart, "NSIS same-version upgrade changed autostart")
        checks["nsis_same_version_upgrade"] = True
        common.invoke([future["nsis"], "/S"], stage="nsis_upgrade")
        assert_nsis(api, future_version, root, files, sentinels, machine_targets)
        require(not (root / "private-fixture/added.txt").exists(), "NSIS higher-version upgrade retained an obsolete file")
        require(autostart_value() == enabled_autostart, "NSIS higher-version upgrade changed autostart")
        common.invoke([setup, "/S"], (1638,), stage="nsis_downgrade")
        assert_nsis(api, future_version, root, files, sentinels, machine_targets)
        checks["downgrade_rejected"] = True
        uninstall_nsis(root)
        assert_removed(root, files, sentinels)
        require(not related_products(api) and not nsis_registrations(), "final uninstall retained a product registration")
        require(autostart_value() is None, "NSIS uninstall retained its owned autostart entry")
        checks["nsis_uninstall"] = True
        common.invoke([setup, "/S"], stage="nsis_install")
        assert_nsis(api, version, root, files, sentinels, machine_targets)
        require(autostart_value() is None, "NSIS clean reinstall automatically enabled autostart")
        write_test_autostart(foreign_autostart)
        uninstall_nsis(root)
        assert_removed(root, files, sentinels)
        require(autostart_value() == foreign_autostart, "NSIS uninstall deleted an unrelated autostart target")
        write_test_autostart(None)
        checks["autostart_lifecycle"] = True
        checks["user_data_preserved"] = True
        checks["per_user_scope"] = True
        require(legacy.base.entries(payload) == files, "lifecycle fixture construction changed the production payload")
        require(legacy.base.digest(msi) == result["msi_sha256"] and legacy.base.digest(setup) == result["setup_sha256"],
                "lifecycle testing modified a production installer")
        require(all(checks.values()), "incomplete Tauri installer lifecycle acceptance")
        result["status"] = "pass"
    finally:
        if result["status"] != "pass" and common._TRACE.document["status"] != "failed":
            common.trace(common._TRACE.document["last_stage"], "error")
        common._TRACE.finish(result["status"] == "pass")
        legacy.base.write_json(report, result)
        common._TRACE = None
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("msi", "setup", "payload", "report"):
        parser.add_argument("--" + argument, type=Path, required=True)
    args = parser.parse_args()
    try:
        lifecycle(args.msi, args.setup, args.payload, args.report)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    print("Tauri MSI and NSIS lifecycle acceptance passed on the disposable native Windows runner")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
