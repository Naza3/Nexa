"""Bounded read-only metadata evidence, not a replacement GGUF validator."""
import hashlib
import struct


def embedded_template_hash(path):
    with path.open("rb") as source:
        consumed = 0

        def read(size):
            nonlocal consumed
            if size < 0 or consumed + size > 32 * 1024 * 1024:
                raise ValueError("metadata_evidence_budget_exceeded")
            consumed += size
            result = source.read(size)
            if len(result) != size:
                raise ValueError("metadata_evidence_truncated")
            return result

        def number(kind):
            return struct.unpack("<" + kind, read(struct.calcsize("<" + kind)))[0]

        def string():
            length = number("Q")
            if length > 4 * 1024 * 1024:
                raise ValueError("metadata_string_limit")
            return read(length)

        def value(kind, depth=0):
            if depth > 4:
                raise ValueError("metadata_array_depth")
            primitives = {0: "B", 1: "b", 2: "H", 3: "h", 4: "I", 5: "i", 6: "f", 7: "?", 10: "Q", 11: "q", 12: "d"}
            if kind in primitives:
                number(primitives[kind])
                return None
            if kind == 8:
                return string()
            if kind == 9:
                subtype, count = number("I"), number("Q")
                if count > 1_000_000:
                    raise ValueError("metadata_array_limit")
                for _ in range(count):
                    value(subtype, depth + 1)
                return None
            raise ValueError("metadata_unknown_type")

        if read(4) != b"GGUF" or number("I") not in (2, 3):
            raise ValueError("metadata_unsupported_header")
        number("Q")  # tensor count; tensor validity belongs to the runtime
        count = number("Q")
        if count > 10_000:
            raise ValueError("metadata_entry_limit")
        result = None
        for _ in range(count):
            key = string()
            item = value(number("I"))
            if key == b"tokenizer.chat_template":
                if result is not None or not isinstance(item, bytes):
                    raise ValueError("metadata_template_identity_invalid")
                item.decode("utf-8")
                result = hashlib.sha256(item).hexdigest()
        if result is None:
            raise ValueError("metadata_template_missing")
        return result
