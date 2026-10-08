"""WiX fragments for Nexa's verified portable payload and read-only guards.

Tauri's official bundler runs candle/light. This module only authors supported
WiX XML; it does not create or modify an MSI database or copy installed files.
"""
from __future__ import annotations

import hashlib
from pathlib import Path, PurePosixPath
import uuid
import xml.etree.ElementTree as ET

import package_windows as base


WIX_NS = "http://schemas.microsoft.com/wix/2006/wi"
UPGRADE_CODE = "85615F8B-FD70-53D9-86ED-4164379CBC40"
MAIN_BINARY = "nexa-desktop.exe"
COMPONENT_GROUP = "NexaPayloadGroup"
NAMESPACE = uuid.UUID(UPGRADE_CODE)
ET.register_namespace("", WIX_NS)


def _element(parent, tag, **attributes):
    return ET.SubElement(parent, "{" + WIX_NS + "}" + tag, attributes)


def _identifier(prefix, path):
    return prefix + hashlib.sha256(path.casefold().encode("ascii")).hexdigest()[:32]


def _component(parent, path, directory, *, cleanup=False):
    identity = ("directory:" if cleanup else "file:") + path.casefold()
    identifier = _identifier("NexaDirectory" if cleanup else "NexaFile", path)
    component = _element(
        parent, "Component", Id=identifier,
        Guid=str(uuid.uuid5(NAMESPACE, "tauri:per-user:x64:" + identity)).upper(),
        Directory=directory, Win64="yes",
    )
    _element(component, "RegistryValue", Root="HKCU",
             Key=r"Software\Nexa\Installer\Components", Name=identifier,
             Type="integer", Value="1", KeyPath="yes")
    return component, identifier


def generate_fragment(payload, files, guard, os_guard, destination):
    """Write one fragment, returning its path for ``wix.fragmentPaths``.

    ``files`` is the already verified portable inventory, including the main
    executable. The Tauri template owns that executable; all remaining files
    are installed at precisely their inventory-relative destination.
    """
    payload = base.resolve_checked_path(Path(payload))
    guard, os_guard = base.regular(guard), base.regular(os_guard)
    destination = base.resolve_checked_path(Path(destination))
    paths, seen = [], set()
    for item in files:
        path = base.relative(item["path"])
        # Keep the existing guard's bounded, ASCII inventory contract.
        if not path.isascii() or len(path) > 180:
            raise ValueError("unsupported WiX payload path")
        if path.casefold() in seen:
            raise ValueError("duplicate WiX payload path")
        seen.add(path.casefold())
        base.regular(payload / path)
        paths.append(path)
    if MAIN_BINARY not in paths:
        raise ValueError("WiX payload is missing the desktop executable")
    if destination.is_relative_to(payload):
        raise ValueError("WiX fragment must be outside the verified payload")
    paths.sort()

    wix = ET.Element("{" + WIX_NS + "}Wix")
    fragment = _element(wix, "Fragment")
    root = _element(fragment, "DirectoryRef", Id="INSTALLDIR")
    directory_nodes = {"": root}
    directory_ids = {"": "INSTALLDIR"}
    for path in paths:
        parent = ""
        for part in PurePosixPath(path).parts[:-1]:
            relative = parent + "/" + part if parent else part
            if relative not in directory_ids:
                directory_ids[relative] = _identifier("NexaDir", relative)
                directory_nodes[relative] = _element(
                    directory_nodes[parent], "Directory",
                    Id=directory_ids[relative], Name=part,
                )
            parent = relative

    group = _element(fragment, "ComponentGroup", Id=COMPONENT_GROUP)
    for path in paths:
        if path == MAIN_BINARY:
            continue
        parent = str(PurePosixPath(path).parent)
        directory = directory_ids["" if parent == "." else parent]
        component, identifier = _component(group, path, directory)
        _element(component, "File", Id="Payload" + identifier,
                 Name=PurePosixPath(path).name, Source=str(payload / path))
    for path, directory in sorted(directory_ids.items()):
        if not path:
            continue  # The template owns INSTALLDIR/Programs cleanup.
        component, identifier = _component(group, path, directory, cleanup=True)
        # RemoveFolder only removes an empty directory. No wildcard deletion,
        # recursive removal, model inventory, or user-data cleanup is authored.
        _element(component, "RemoveFolder", Id="Empty" + identifier, On="uninstall")

    table = _element(fragment, "CustomTable", Id="NexaPayload")
    _element(table, "Column", Id="Path", Type="string", Width="255", PrimaryKey="yes")
    for path in paths:
        row = _element(table, "Row")
        _element(row, "Data", Column="Path").text = path.replace("/", "\\")

    _element(fragment, "Binary", Id="NexaGuardBinary", SourceFile=str(guard))
    _element(fragment, "Binary", Id="NexaOsGuardBinary", SourceFile=str(os_guard))
    _element(fragment, "CustomAction", Id="NexaOsGuard", BinaryKey="NexaOsGuardBinary",
             ExeCommand="", Execute="immediate", Return="check")
    _element(fragment, "CustomAction", Id="NexaGuard", BinaryKey="NexaGuardBinary",
             DllEntry="NexaGuard", Execute="immediate", Return="check")
    _element(fragment, "CustomAction", Id="NexaRemoveAutostart", BinaryKey="NexaGuardBinary",
             DllEntry="NexaRemoveAutostart", Execute="commit", Impersonate="yes", Return="check")
    # Preserve the established native guard API, aliasing Tauri's directory IDs
    # only after MSI has resolved the directories in each execution context.
    _element(fragment, "SetProperty", Id="INSTALLFOLDER", Value="[INSTALLDIR]",
             After="CostFinalize", Sequence="both")
    _element(fragment, "SetProperty", Id="NexaMenuFolder", Value="[ApplicationProgramsFolder]",
             After="SetINSTALLFOLDER", Sequence="both")
    for sequence in ("InstallUISequence", "InstallExecuteSequence"):
        actions = _element(fragment, sequence)
        _element(actions, "Custom", Action="NexaOsGuard", After="SetNexaMenuFolder").text = "1"
        _element(actions, "Custom", Action="NexaGuard", After="NexaOsGuard").text = "1"
        if sequence == "InstallExecuteSequence":
            _element(actions, "Custom", Action="NexaRemoveAutostart", Before="InstallFinalize").text = (
                'REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE'
            )

    ET.indent(wix, space="  ")
    destination.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(wix).write(destination, encoding="utf-8", xml_declaration=True)
    return destination
