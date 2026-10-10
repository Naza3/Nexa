"""Offline metadata evidence bounds; no model inference."""
import hashlib
from pathlib import Path
import struct
import tempfile
import unittest
from model_identity import embedded_template_hash


def text(value):
    return struct.pack("<Q", len(value)) + value


def fixture(template=b"{{ messages }}"):
    return b"GGUF" + struct.pack("<IQQ", 3, 0, 1) + text(b"tokenizer.chat_template") + struct.pack("<I", 8) + text(template)


class IdentityTests(unittest.TestCase):
    def check_bytes(self, data):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "fixture.gguf"
            path.write_bytes(data)
            return embedded_template_hash(path)

    def test_template_identity(self):
        self.assertEqual(self.check_bytes(fixture()), hashlib.sha256(b"{{ messages }}").hexdigest())

    def test_truncation_and_missing_template_fail(self):
        for data in (fixture()[:-1], b"GGUF" + struct.pack("<IQQ", 3, 0, 0)):
            with self.assertRaises(ValueError):
                self.check_bytes(data)

    def test_oversized_declared_string_fails_before_read(self):
        data = b"GGUF" + struct.pack("<IQQQ", 3, 0, 1, 5 * 1024 * 1024)
        with self.assertRaises(ValueError):
            self.check_bytes(data)


if __name__ == "__main__":
    unittest.main()
