"""Maintenance safety checks inspect registration context, not just return codes."""
from contextlib import contextmanager
import ctypes
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import package_windows_msi as pack
import test_windows_msi_lifecycle as lifecycle
from windows_msi_api import Msi
import windows_installer_diagnostics as diag


class InstallerContextTests(unittest.TestCase):
    def enum_api(self, contexts, terminal=259):
        api = object.__new__(Msi)
        product = pack.product_code("0.1.0")
        calls = []
        def enumerate_product(code, sid, allowed, index, output, context, output_sid, sid_length):
            calls.append((code, sid, allowed, index, output_sid, sid_length))
            if index >= len(contexts): return terminal
            output.value = product
            context._obj.value = contexts[index]
            return 0
        api.MsiEnumProductsExW = enumerate_product
        return api, product, calls

    def test_enumeration_reads_current_user_and_machine_without_sids(self):
        api, product, calls = self.enum_api([2, 4])
        self.assertEqual(api.product_contexts(product), [2, 4])
        self.assertEqual(len(calls), 3)
        for _, sid, context, _, returned_sid, size in calls:
            self.assertIsNone(sid)
            self.assertEqual(context, 7)
            self.assertIsNone(returned_sid)
            self.assertIsNone(size)

    def test_enumeration_errors_and_unbounded_results_fail_closed(self):
        api, product, _ = self.enum_api([], terminal=5)
        with self.assertRaisesRegex(ValueError, "failed: 5"): api.product_contexts(product)
        api, product, _ = self.enum_api([9])
        with self.assertRaises(ValueError): api.product_contexts(product)
        api, product, _ = self.enum_api([2] * 8)
        with self.assertRaisesRegex(ValueError, "bound"): api.product_contexts(product)
        api, product, _ = self.enum_api([], terminal=1605)
        self.assertEqual(api.product_contexts(product), [])

    def test_only_installed_unmanaged_user_context_is_accepted(self):
        api = mock.Mock()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(lifecycle, "installed_payload") as payload, mock.patch.object(lifecycle, "same_sentinels") as sentinels:
            root = Path(temporary)
            target = root / "machine-target"
            for contexts, state in (([], -1), ([2], 1), ([1], 5), ([4], 5), ([2, 4], 5), ([1, 2], 5)):
                api.product_contexts.return_value = contexts
                api.MsiQueryProductStateW.return_value = state
                with self.subTest(contexts=contexts, state=state), self.assertRaises(ValueError):
                    lifecycle.per_user_unchanged(api, "0.1.0", root, [], {}, [target])
            payload.assert_not_called(); sentinels.assert_not_called()
            api.product_contexts.return_value = [2]; api.MsiQueryProductStateW.return_value = 5
            self.assertEqual(lifecycle.per_user_unchanged(api, "0.1.0", root, [], {}, [target]), [2])
            payload.assert_called_once(); sentinels.assert_called_once()
            target.mkdir()
            with self.assertRaisesRegex(ValueError, "machine-wide"):
                lifecycle.per_user_unchanged(api, "0.1.0", root, [], {}, [target])

    def test_reboot_fixture_refreshes_and_reads_back_package_code(self):
        original = "{00000000-0000-0000-0000-000000000001}"
        class Api:
            code = original
            @contextmanager
            def database(self, path, mode=0): yield 1
            def summary_string(self, database, key): return self.code
            def execute(self, *args): pass
            def summary(self, database, values): self.code = values[9]
            def MsiDatabaseCommit(self, database): return 0
            def check(self, code, action): assert code == 0
        api = Api()
        with mock.patch.object(lifecycle, "Msi", return_value=api):
            changed = lifecycle.add_reboot_fixture(Path("private-fixture.msi"), original)
        self.assertNotEqual(changed, original)
        self.assertEqual(changed, api.code)

    def test_source_retains_fresh_rejection_and_binds_maintenance_state(self):
        source = Path(lifecycle.__file__).read_text(encoding="utf-8")
        self.assertIn('("fresh-wrong-scope", ["ALLUSERS=1"])', source)
        self.assertIn('logs / (label + ".log"), *properties, accepted=(1603,)', source)
        self.assertIn('trace("maintenance_scope_verified"', source)
        self.assertIn('trace("maintenance_directory_verified"', source)
        self.assertIn('require(not destination.exists(), "installer accepted directory override")', source)
        self.assertIn('msi_fresh_wrong_scope', diag.STAGES)
        self.assertIn('msi_fresh_wrong_directory', diag.STAGES)


if __name__ == "__main__": unittest.main()
