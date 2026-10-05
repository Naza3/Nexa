"""Lossless license presentation and fail-closed bundle contract tests."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import package_windows as pack


def original_fixture(stage, html=True, npm=False):
    originals = {
        "licenses/rust-crates/example-1.2.3/LICENSE": b"Copyright Example\r\nPermission is hereby granted.\r\n",
        "licenses/rust-crates/example-1.2.3/NOTICE": b"Keep this special NOTICE and attribution.\n",
        "licenses/rust-crates/other-4.5.6/LICENSE": b"Copyright Example\r\nPermission is hereby granted.\r\n",
    }
    if html:
        originals["licenses/rust-std/COPYRIGHT-library.html"] = b"<!doctype html><html><body>Original copyright HTML</body></html>\n"
    for name, raw in originals.items():
        target = stage / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(raw)
    pack.write_json(stage / pack.LICENSE_INDEX, {"scope": "normal/build closure fixture", "files": [
        {"path": name, "sha256": hashlib.sha256(raw).hexdigest(), "component": name.split("/")[-2],
         "source": "https://example.invalid/locked-source", "version": "1.2.3"}
        for name, raw in sorted(originals.items())]})
    if npm:
        name = "licenses/npm/react/LICENSE"
        (stage / name).parent.mkdir(parents=True)
        (stage / name).write_text("Original React license fixture\n", encoding="utf-8")
        pack.write_json(stage / "licenses/npm-index.json", {"scope": "npm production fixture", "files": [
            {"path": name, "sha256": pack.digest(stage / name), "component": "react", "version": "1.0.0",
             "integrity": "sha512-fixture", "source": "https://example.invalid/react"}]})
    return {"licenses/" + item["path"]: (stage / "licenses" / item["path"]).read_bytes()
            for item in pack.entries(stage / "licenses")}


def compact_fixture(stage, html=True, npm=False):
    originals = original_fixture(stage, html=html, npm=npm)
    pack.consolidate_licenses(stage)
    return originals


def crt_original_fixture(stage, suffix=".docx"):
    name = "licenses/microsoft-crt/original-3-vs2022-license" + suffix
    raw = b"PK\x03\x04\x00\xfforiginal Microsoft binary fixture" if suffix == ".docx" else (
        b"%PDF-1.7\noriginal Microsoft binary fixture\x00" if suffix == ".pdf" else "original UTF-16 terms".encode("utf-16"))
    (stage / name).parent.mkdir(parents=True, exist_ok=True)
    (stage / name).write_bytes(raw)
    pack.write_json(stage / "licenses/microsoft-crt/index.json", {"files": [
        {"path": name, "sha256": hashlib.sha256(raw).hexdigest(), "source_url": "https://example.invalid/official-license"}]})
    return name, raw


class LicenseBundleTests(unittest.TestCase):
    def test_every_original_byte_and_attribution_survives(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            original = compact_fixture(stage, npm=True)
            result = pack.verify_license_bundle(stage)
            self.assertEqual({name: item["raw"] for name, item in result.items()}, original)
            self.assertEqual(len(pack.entries(stage / "licenses")), 3)
            self.assertEqual((stage / pack.LICENSE_HTML).read_bytes(), original["licenses/rust-std/COPYRIGHT-library.html"])
            self.assertIn(b"Keep this special NOTICE", (stage / pack.LICENSE_TEXT).read_bytes())
            self.assertIn(b"\r\nPermission", (stage / pack.LICENSE_TEXT).read_bytes())
            npm = result["licenses/npm/react/LICENSE"]["document"]["attributions"][0]
            self.assertEqual(npm["component"], "react")
            self.assertEqual(npm["integrity"], "sha512-fixture")
            # Identical licenses remain assigned to each original component.
            self.assertEqual(result["licenses/rust-crates/example-1.2.3/LICENSE"]["raw"],
                             result["licenses/rust-crates/other-4.5.6/LICENSE"]["raw"])

    def test_no_html_requires_only_two_files(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            compact_fixture(stage, html=False)
            self.assertEqual({item["path"] for item in pack.entries(stage / "licenses")}, {"index.json", "THIRD_PARTY_LICENSES.txt"})

    def test_consolidation_is_deterministic(self):
        with tempfile.TemporaryDirectory() as folder:
            roots = [Path(folder) / str(n) for n in range(2)]
            for root in roots:
                root.mkdir()
                compact_fixture(root, npm=True)
            self.assertEqual(pack.entries(roots[0]), pack.entries(roots[1]))

    def test_original_closure_failures_do_not_remove_input(self):
        for mode in ("hash", "missing", "duplicate", "unindexed", "binary", "invalid-utf8", "duplicate-json-key"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                original_fixture(stage)
                path = stage / pack.LICENSE_INDEX
                index = json.loads(path.read_text(encoding="utf-8"))
                if mode == "hash":
                    index["files"][0]["sha256"] = "0" * 64
                elif mode == "missing":
                    (stage / index["files"][0]["path"]).unlink()
                elif mode == "duplicate":
                    index["files"].append(index["files"][0].copy())
                elif mode == "unindexed":
                    (stage / "licenses/extra.txt").write_text("unmapped", encoding="utf-8")
                elif mode in ("binary", "invalid-utf8"):
                    file = stage / index["files"][0]["path"]
                    file.write_bytes(b"binary\x00original" if mode == "binary" else b"invalid\xfforiginal")
                    index["files"][0]["sha256"] = pack.digest(file)
                pack.write_json(path, index)
                if mode == "duplicate-json-key":
                    path.write_text('{"files": [], "files": []}', encoding="utf-8")
                before = pack.entries(stage)
                with self.assertRaises(ValueError):
                    pack.consolidate_licenses(stage)
                self.assertEqual(pack.entries(stage), before)

    def test_bundle_rejects_semantic_tampering_even_without_outer_manifest(self):
        modes = ("changed", "trailing", "header", "footer", "offset", "huge-range", "negative", "bool-size", "float-size",
                 "duplicate", "case-duplicate", "reordered", "missing", "storage-escape", "attribution",
                 "empty-attribution", "empty-index", "unknown-field", "html-changed", "html-offset", "extra-file", "empty-dir", "duplicate-json-key")
        for mode in modes:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                compact_fixture(stage, npm=True)
                path = stage / pack.LICENSE_INDEX
                index = json.loads(path.read_text(encoding="utf-8"))
                document = next(d for d in index["documents"] if d["original_path"].endswith("example-1.2.3/LICENSE"))
                text_path = stage / pack.LICENSE_TEXT
                raw = text_path.read_bytes()
                if mode == "changed":
                    offset = document["offset_bytes"]
                    text_path.write_bytes(raw[:offset] + b"x" + raw[offset + 1:])
                elif mode == "trailing":
                    text_path.write_bytes(raw + b"hidden payload")
                elif mode == "header":
                    text_path.write_bytes(raw.replace(b"BEGIN ORIGINAL", b"WRONG ORIGINAL", 1))
                elif mode == "footer":
                    text_path.write_bytes(raw.replace(pack.LICENSE_FOOTER, b"\nWRONG FOOTER\n", 1))
                elif mode == "offset":
                    document["offset_bytes"] += 1
                elif mode == "huge-range":
                    document["size_bytes"] = 2**80
                elif mode == "negative":
                    document["offset_bytes"] = -1
                elif mode == "bool-size":
                    document["size_bytes"] = True
                elif mode == "float-size":
                    document["size_bytes"] = float(document["size_bytes"])
                elif mode == "duplicate":
                    index["documents"].append(document.copy())
                elif mode == "case-duplicate":
                    duplicate = document.copy()
                    duplicate["original_path"] = document["original_path"].upper()
                    index["documents"].append(duplicate)
                elif mode == "reordered":
                    index["documents"].reverse()
                elif mode == "missing":
                    index["documents"].remove(document)
                elif mode == "storage-escape":
                    document["stored_path"] = "../outside.txt"
                elif mode == "attribution":
                    document["attributions"][0]["component"] = "false component"
                elif mode == "empty-attribution":
                    document["attributions"] = []
                elif mode == "empty-index":
                    index["documents"] = []
                elif mode == "unknown-field":
                    document["unsupported"] = 1
                elif mode == "html-changed":
                    (stage / pack.LICENSE_HTML).write_text("changed", encoding="utf-8")
                elif mode == "html-offset":
                    next(d for d in index["documents"] if d["stored_path"] == pack.LICENSE_HTML)["offset_bytes"] = 1
                elif mode == "extra-file":
                    (stage / "licenses/another-NOTICE.txt").write_text("extra", encoding="utf-8")
                elif mode == "empty-dir":
                    (stage / "licenses/untracked").mkdir()
                pack.write_json(path, index)
                if mode == "duplicate-json-key":
                    path.write_text(path.read_text(encoding="utf-8").replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1'), encoding="utf-8")
                with self.assertRaises(ValueError):
                    pack.verify_license_bundle(stage)

    def test_empty_bundle_and_reconsolidation_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            (stage / "licenses").mkdir()
            with self.assertRaises(ValueError):
                pack.consolidate_licenses(stage)
            compact_fixture(stage)
            before = pack.entries(stage)
            with self.assertRaises(ValueError):
                pack.consolidate_licenses(stage)
            self.assertEqual(pack.entries(stage), before)

    def test_license_json_requires_utf8_without_bom_and_standard_numbers(self):
        for raw in ('{"files":[]}'.encode("utf-16"), b'\xef\xbb\xbf{"files":[]}',
                    b'{"unsupported":NaN}', b'{"unsupported":Infinity}', b'{"unsupported":-Infinity}'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                pack.strict_json(raw)

    def test_license_json_rejects_rounded_numbers_and_retains_exact_integer_limits(self):
        for number in ("18446744073709551616", "18446744073709551617", "-9223372036854775809", "1.5", "1.0", "1e0", "-0"):
            for template in ('{"number":%s}', '{"attributions":[{"nested":{"serial":%s}}]}'):
                with self.subTest(number=number, template=template), self.assertRaises(ValueError):
                    pack.strict_json(template % number)
            with self.subTest(inventory_number=number), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                original_fixture(stage)
                path = stage / pack.LICENSE_INDEX
                raw = path.read_text(encoding="utf-8")
                changed = raw.replace('"component":', '"nested":{"serial":' + number + '},"component":', 1)
                self.assertNotEqual(raw, changed)
                path.write_text(changed, encoding="utf-8")
                before = pack.entries(stage)
                with self.assertRaises(ValueError):
                    pack.consolidate_licenses(stage)
                self.assertEqual(pack.entries(stage), before)
        self.assertEqual(pack.strict_json('{"values":[0,-1,18446744073709551615,-9223372036854775808]}'),
                         {"values": [0, -1, 2**64 - 1, -(2**63)]})

    def test_boolean_and_integer_attribution_fields_are_not_equivalent(self):
        for original, changed in ((True, 1), (False, 0), (1, True), (0, False)):
            with self.subTest(original=original, changed=changed), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                original_fixture(stage)
                path = stage / pack.LICENSE_INDEX
                index = pack.strict_json(path.read_bytes())
                record_path = index["files"][0]["path"]
                index["files"][0]["nested"] = {"serial": original}
                pack.write_json(path, index)
                pack.consolidate_licenses(stage)
                index = pack.strict_json(path.read_bytes())
                document = next(document for document in index["documents"] if document["original_path"] == record_path)
                document["attributions"][0]["nested"]["serial"] = changed
                pack.write_json(path, index)
                with self.assertRaisesRegex(ValueError, "attribution differs"):
                    pack.verify_license_bundle(stage)

    def test_rust_copyright_cannot_be_moved_into_plain_text(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            compact_fixture(stage)
            index = json.loads((stage / pack.LICENSE_INDEX).read_text(encoding="utf-8"))
            original = pack.verify_license_bundle(stage)
            bundle = bytearray(pack.LICENSE_PREFIX)
            for document in index["documents"]:
                document["stored_path"] = pack.LICENSE_TEXT
                bundle.extend(pack.license_header(document))
                document["offset_bytes"] = len(bundle)
                bundle.extend(original[document["original_path"]]["raw"])
                bundle.extend(pack.LICENSE_FOOTER)
            pack.write_json(stage / pack.LICENSE_INDEX, index)
            (stage / pack.LICENSE_TEXT).write_bytes(bundle)
            (stage / pack.LICENSE_HTML).unlink()
            with self.assertRaisesRegex(ValueError, "remain directly readable"):
                pack.verify_license_bundle(stage)

    def test_binary_microsoft_originals_and_bundled_notices_are_lossless(self):
        for suffix in (".docx", ".pdf", ".txt"):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                original_fixture(stage)
                name, binary = crt_original_fixture(stage, suffix)
                notice = b"Original root notices\r\nDo not drop copyright or NOTICE.\n"
                (stage / pack.LICENSE_NOTICES).write_bytes(notice)
                before = {item["path"]: (stage / item["path"]).read_bytes() for item in pack.entries(stage)}
                pack.consolidate_licenses(stage)
                result = pack.verify_license_bundle(stage)
                self.assertEqual({name: item["raw"] for name, item in result.items()}, before)
                self.assertEqual(result[pack.LICENSE_NOTICES]["raw"], notice)
                self.assertFalse((stage / pack.LICENSE_NOTICES).exists())
                document = result[name]["document"]
                self.assertEqual(document["stored_path"], "licenses/ORIGINAL-" + Path(name).name)
                self.assertEqual(document["offset_bytes"], 0)
                self.assertEqual((stage / document["stored_path"]).read_bytes(), binary)
                self.assertEqual(len(pack.license_related_files(stage)), 4)
                (stage / document["stored_path"]).write_bytes(binary + b"changed")
                with self.assertRaises(ValueError):
                    pack.verify_license_bundle(stage)

    def test_runtime_fixture_rejects_attribution_drift_for_every_original(self):
        fixture = json.loads((pack.ROOT / "scripts/fixtures/license-bundle/runtime-contract.json").read_text(encoding="utf-8"))
        for position, document in enumerate(fixture["index"]["documents"]):
            original = document["attributions"]
            mutations = [[], [None], [{}], [False], ["wrong"], [[]], original + original]
            extra = json.loads(json.dumps(original))
            extra[0]["unrecorded"] = True
            mutations.append(extra)
            for key in original[0]:
                changed = json.loads(json.dumps(original))
                changed[0][key] = "wrong"
                mutations.append(changed)
                missing = json.loads(json.dumps(original))
                del missing[0][key]
                mutations.append(missing)
            for attribution in mutations:
                with self.subTest(original=document["original_path"], attribution=attribution), tempfile.TemporaryDirectory() as folder:
                    stage = Path(folder)
                    (stage / "licenses").mkdir()
                    index = json.loads(json.dumps(fixture["index"]))
                    index["documents"][position]["attributions"] = attribution
                    pack.write_json(stage / pack.LICENSE_INDEX, index)
                    for name, raw in fixture["stored_files"].items():
                        (stage / name).write_bytes(bytes(raw))
                    with self.assertRaises(ValueError):
                        pack.verify_license_bundle(stage)

    def test_python_rust_runtime_fixture_retains_notice_and_binary_original(self):
        fixture = json.loads((pack.ROOT / "scripts/fixtures/license-bundle/runtime-contract.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            (stage / "licenses").mkdir()
            pack.write_json(stage / pack.LICENSE_INDEX, fixture["index"])
            for name, raw in fixture["stored_files"].items():
                (stage / name).write_bytes(bytes(raw))
            originals = pack.verify_license_bundle(stage)
            self.assertEqual(originals[pack.LICENSE_NOTICES]["raw"], "Synthetic Nexa root notices\r\n版权\nEND ORIGINAL\n".encode("utf-8"))
            recreated = stage / "recreated"
            for name, item in originals.items():
                (recreated / name).parent.mkdir(parents=True, exist_ok=True)
                (recreated / name).write_bytes(item["raw"])
            self.assertEqual(pack.consolidate_licenses(recreated), fixture["index"])
            self.assertEqual({name: list((recreated / name).read_bytes()) for name in fixture["stored_files"]}, fixture["stored_files"])


if __name__ == "__main__":
    unittest.main()
