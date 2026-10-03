//! Bounded, native-free GGUF structural inspection.
//!
//! The metadata walk follows `xtask/src/gguf.rs`; this production reader also
//! validates tensor descriptors, extents, and non-overlap. None of these checks
//! establish architecture support or successful inference.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};

use crate::{Result, invalid_manifest, io_error};

const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STRING_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: u64 = 100_000;

#[derive(Debug, PartialEq)]
pub(crate) struct Metadata {
    pub architecture: String,
    pub file_type: u32,
    pub template: String,
    pub context_length: u32,
}

pub(crate) fn read<R: Read + Seek>(reader: R) -> Result<Metadata> {
    let mut input = Input::new(reader)?;
    if input.bytes::<4>()? != *b"GGUF" {
        return Err(invalid_manifest("invalid GGUF magic"));
    }
    if !matches!(input.u32()?, 2 | 3) {
        return Err(invalid_manifest("unsupported GGUF version"));
    }
    let tensors = input.u64()?;
    let count = input.u64()?;
    if tensors == 0 || tensors > MAX_ENTRIES || count > MAX_ENTRIES {
        return Err(invalid_manifest(
            "GGUF entry count outside inspection bounds",
        ));
    }
    let mut strings = BTreeMap::new();
    let mut numbers = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let key = input.string(4096)?;
        if key.is_empty() || key.contains('\0') || !seen.insert(key.clone()) {
            return Err(invalid_manifest(
                "empty, NUL-containing or duplicate GGUF metadata key",
            ));
        }
        let kind = input.u32()?;
        match key.as_str() {
            "general.architecture" | "tokenizer.chat_template" => {
                if kind != 8 {
                    return Err(invalid_manifest(
                        "GGUF architecture/template must be a string",
                    ));
                }
                strings.insert(key, input.string(MAX_STRING_BYTES)?);
            }
            "split.count" | "split.no" => {
                // llama.cpp otherwise opens sibling shards outside this file's
                // hash and Windows lease. A multi-file closure is not supported.
                let value = match kind {
                    2 => u32::from(u16::from_le_bytes(input.bytes()?)),
                    4 => input.u32()?,
                    _ => {
                        return Err(invalid_manifest(
                            "GGUF split metadata must be unsigned integer",
                        ));
                    }
                };
                if (key == "split.count" && value > 1) || (key == "split.no" && value != 0) {
                    return Err(runtime_types::RuntimeError::new(
                        runtime_types::ErrorCode::UnsupportedModel,
                        "multi-file GGUF is unsupported; every loaded byte must belong to the protected single file",
                    ));
                }
            }
            "general.file_type" | "general.alignment" => {
                if kind != 4 {
                    return Err(invalid_manifest("GGUF file type/alignment must be uint32"));
                }
                numbers.insert(key, input.u32()?);
            }
            _ if key.ends_with(".context_length") => {
                if kind != 4 {
                    return Err(invalid_manifest("GGUF context length must be uint32"));
                }
                numbers.insert(key, input.u32()?);
            }
            _ => input.skip_value(kind)?,
        }
    }
    let alignment = u64::from(numbers.get("general.alignment").copied().unwrap_or(32));
    if !alignment.is_power_of_two() || alignment > 4096 {
        return Err(invalid_manifest(
            "GGUF alignment must be a power of two up to 4096",
        ));
    }
    let mut names = BTreeSet::new();
    let mut extents = Vec::with_capacity(tensors as usize);
    for _ in 0..tensors {
        let name = input.string(4096)?;
        if name.is_empty() || name.contains('\0') || !names.insert(name) {
            return Err(invalid_manifest(
                "empty, NUL-containing or duplicate GGUF tensor name",
            ));
        }
        let dimensions = input.u32()?;
        if !(1..=4).contains(&dimensions) {
            return Err(invalid_manifest(
                "GGUF tensor dimension count outside 1..=4",
            ));
        }
        let mut elements = 1_u64;
        let mut first_dimension = 0;
        for dimension in 0..dimensions {
            let size = input.u64()?;
            if dimension == 0 {
                first_dimension = size;
            }
            if size == 0 || size > i64::MAX as u64 {
                return Err(invalid_manifest("invalid GGUF tensor dimension"));
            }
            elements = elements
                .checked_mul(size)
                .ok_or_else(|| invalid_manifest("GGUF tensor dimension overflow"))?;
        }
        let (block, bytes) = tensor_layout(input.u32()?)?;
        if first_dimension % block != 0 {
            return Err(invalid_manifest(
                "GGUF tensor row is not a whole quantization block",
            ));
        }
        let length = (elements / block)
            .checked_mul(bytes)
            .ok_or_else(|| invalid_manifest("GGUF tensor size overflow"))?;
        let offset = input.u64()?;
        if offset % alignment != 0 {
            return Err(invalid_manifest("unaligned GGUF tensor offset"));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| invalid_manifest("GGUF tensor extent overflow"))?;
        extents.push((offset, end));
    }
    let data_start = input
        .position
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| invalid_manifest("GGUF header alignment overflow"))?;
    let data_length = input
        .length
        .checked_sub(data_start)
        .ok_or_else(|| invalid_manifest("GGUF tensor data missing"))?;
    extents.sort_unstable();
    let mut previous_end = 0;
    for (start, end) in extents {
        if start < previous_end || end > data_length {
            return Err(invalid_manifest(
                "GGUF tensor data overlaps or is truncated",
            ));
        }
        previous_end = end;
    }
    let architecture = strings
        .remove("general.architecture")
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or_else(|| invalid_manifest("GGUF architecture missing or invalid"))?;
    let context_length = *numbers
        .get(&format!("{architecture}.context_length"))
        .filter(|&&value| value >= 32)
        .ok_or_else(|| invalid_manifest("GGUF context length missing or invalid"))?;
    Ok(Metadata {
        context_length,
        architecture,
        file_type: *numbers
            .get("general.file_type")
            .ok_or_else(|| invalid_manifest("GGUF file type missing"))?,
        template: strings
            .remove("tokenizer.chat_template")
            // The native template API is C-string based. Reject NUL rather
            // than hashing one template and executing a silently truncated one.
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or_else(|| {
                runtime_types::RuntimeError::new(
                    runtime_types::ErrorCode::UnsupportedChatTemplate,
                    "GGUF requires a nonempty, NUL-free embedded chat template; no fallback is provided",
                )
            })?,
    })
}

/// Conservative supported structural layouts from the locked ggml.h and
/// ggml-common.h. New layouts fail closed until their extents are implemented.
fn tensor_layout(kind: u32) -> Result<(u64, u64)> {
    match kind {
        0 | 26 => Ok((1, 4)),
        1 | 25 | 30 => Ok((1, 2)),
        2 => Ok((32, 18)),
        3 => Ok((32, 20)),
        6 => Ok((32, 22)),
        7 => Ok((32, 24)),
        8 => Ok((32, 34)),
        9 => Ok((32, 36)),
        10 => Ok((256, 84)),
        11 => Ok((256, 110)),
        12 => Ok((256, 144)),
        13 => Ok((256, 176)),
        14 => Ok((256, 210)),
        15 => Ok((256, 292)),
        24 => Ok((1, 1)),
        27 | 28 => Ok((1, 8)),
        _ => Err(invalid_manifest(
            "GGUF tensor type is outside the structural inspection subset",
        )),
    }
}

struct Input<R> {
    reader: R,
    position: u64,
    length: u64,
    limit: u64,
}
impl<R: Read + Seek> Input<R> {
    fn new(mut reader: R) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
        reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
        Ok(Self {
            reader,
            position: 0,
            length,
            limit: length.min(MAX_HEADER_BYTES),
        })
    }
    fn reserve(&mut self, length: u64) -> Result<()> {
        self.position = self
            .position
            .checked_add(length)
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| invalid_manifest("truncated or oversized GGUF header"))?;
        Ok(())
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.reserve(N as u64)?;
        let mut bytes = [0; N];
        self.reader.read_exact(&mut bytes).map_err(io_error)?;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn string(&mut self, maximum: u64) -> Result<String> {
        let length = self.u64()?;
        if length > maximum {
            return Err(invalid_manifest("GGUF string exceeds inspection limit"));
        }
        self.reserve(length)?;
        let mut bytes = vec![0; length as usize];
        self.reader.read_exact(&mut bytes).map_err(io_error)?;
        String::from_utf8(bytes).map_err(|_| invalid_manifest("GGUF string is not UTF-8"))
    }
    fn skip(&mut self, length: u64) -> Result<()> {
        self.reserve(length)?;
        self.reader
            .seek(SeekFrom::Start(self.position))
            .map_err(io_error)?;
        Ok(())
    }
    fn skip_value(&mut self, kind: u32) -> Result<()> {
        match kind {
            8 => {
                let length = self.u64()?;
                self.skip(length)
            }
            9 => {
                let element = self.u32()?;
                let count = self.u64()?;
                if count > 1_000_000 || element == 9 {
                    return Err(invalid_manifest("unsupported or oversized GGUF array"));
                }
                if element == 8 {
                    for _ in 0..count {
                        self.skip_value(element)?;
                    }
                    Ok(())
                } else {
                    self.skip(
                        count
                            .checked_mul(scalar_size(element)?)
                            .ok_or_else(|| invalid_manifest("GGUF array overflow"))?,
                    )
                }
            }
            _ => self.skip(scalar_size(kind)?),
        }
    }
}
fn scalar_size(kind: u32) -> Result<u64> {
    match kind {
        0 | 1 | 7 => Ok(1),
        2 | 3 => Ok(2),
        4..=6 => Ok(4),
        10..=12 => Ok(8),
        _ => Err(invalid_manifest("unknown GGUF metadata type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    fn header(tensors: u64, extra: Option<(&str, u32, Vec<u8>)>) -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(tensors.to_le_bytes());
        bytes.extend((4_u64 + u64::from(extra.is_some())).to_le_bytes());
        for (key, value) in [
            ("general.architecture", "qwen3"),
            ("tokenizer.chat_template", "template\n中文"),
        ] {
            string(&mut bytes, key);
            bytes.extend(8_u32.to_le_bytes());
            string(&mut bytes, value);
        }
        for (key, value) in [
            ("general.file_type", 7_u32),
            ("qwen3.context_length", 40960),
        ] {
            string(&mut bytes, key);
            bytes.extend(4_u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        if let Some((key, kind, value)) = extra {
            string(&mut bytes, key);
            bytes.extend(kind.to_le_bytes());
            bytes.extend(value);
        }
        bytes
    }
    fn tensor(bytes: &mut Vec<u8>, name: &str, shape: &[u64], kind: u32, offset: u64) {
        string(bytes, name);
        bytes.extend((shape.len() as u32).to_le_bytes());
        for dimension in shape {
            bytes.extend(dimension.to_le_bytes());
        }
        bytes.extend(kind.to_le_bytes());
        bytes.extend(offset.to_le_bytes());
    }
    fn finish(mut bytes: Vec<u8>, payload: usize) -> Vec<u8> {
        bytes.resize(bytes.len().next_multiple_of(32), 0);
        bytes.resize(bytes.len() + payload, 0);
        bytes
    }
    #[test]
    fn embedded_template_nul_cannot_change_native_template_identity() {
        let mut bytes = header(1, None);
        tensor(&mut bytes, "weight", &[1], 0, 0);
        let mut bytes = finish(bytes, 4);
        let value = bytes
            .windows(b"template\n".len())
            .position(|part| part == b"template\n")
            .unwrap();
        bytes[value] = 0;
        assert_eq!(
            read(Cursor::new(bytes)).unwrap_err().code,
            runtime_types::ErrorCode::UnsupportedChatTemplate
        );
    }

    #[test]
    fn nul_keys_and_tensor_names_fail_without_banning_ordinary_values() {
        let mut bytes = header(1, None);
        let key = bytes
            .windows(b"general.architecture".len())
            .position(|part| part == b"general.architecture")
            .unwrap();
        bytes[key] = 0;
        tensor(&mut bytes, "weight", &[1], 0, 0);
        assert_eq!(
            read(Cursor::new(finish(bytes, 4))).unwrap_err().code,
            runtime_types::ErrorCode::InvalidManifest
        );
        let mut bytes = header(1, None);
        tensor(&mut bytes, "wei\0ght", &[1], 0, 0);
        assert_eq!(
            read(Cursor::new(finish(bytes, 4))).unwrap_err().code,
            runtime_types::ErrorCode::InvalidManifest
        );
        let mut value = Vec::new();
        string(&mut value, "ordinary\0value");
        let mut bytes = header(1, Some(("tokenizer.ordinary_value", 8, value)));
        tensor(&mut bytes, "weight", &[1], 0, 0);
        assert!(read(Cursor::new(finish(bytes, 4))).is_ok());
    }

    #[test]
    fn split_models_cannot_expand_the_verified_file_closure() {
        for (key, kind, value) in [
            ("split.count", 2, 2_u16.to_le_bytes().to_vec()),
            ("split.count", 4, 2_u32.to_le_bytes().to_vec()),
            ("split.no", 2, 1_u16.to_le_bytes().to_vec()),
            ("split.no", 4, 1_u32.to_le_bytes().to_vec()),
        ] {
            let mut bytes = header(1, Some((key, kind, value)));
            tensor(&mut bytes, "weight", &[1], 0, 0);
            assert_eq!(
                read(Cursor::new(finish(bytes, 4))).unwrap_err().code,
                runtime_types::ErrorCode::UnsupportedModel
            );
        }
        for (key, value) in [("split.count", 1_u16), ("split.no", 0_u16)] {
            let mut bytes = header(1, Some((key, 2, value.to_le_bytes().to_vec())));
            tensor(&mut bytes, "weight", &[1], 0, 0);
            assert!(read(Cursor::new(finish(bytes, 4))).is_ok());
        }
    }

    #[test]
    fn valid_metadata_and_disjoint_tensor_extents() {
        let mut bytes = header(2, None);
        tensor(&mut bytes, "weights", &[32], 8, 0); // 34-byte Q8_0 block
        tensor(&mut bytes, "bias", &[1], 0, 64);
        let metadata = read(Cursor::new(finish(bytes, 68))).unwrap();
        assert_eq!(metadata.template, "template\n中文");
        assert_eq!(metadata.context_length, 40960);
    }
    #[test]
    fn rejects_tensor_overflow_unknown_layout_alignment_and_overlap() {
        for (shape, kind, offset) in [
            (vec![], 0, 0),
            (vec![1; 5], 0, 0),
            (vec![0], 0, 0),
            (vec![u64::MAX], 0, 0),
            (vec![i64::MAX as u64, 3], 0, 0),
            (vec![1], 99, 0),
            (vec![1], 0, 1),
            (vec![31], 8, 0),
            (vec![32], 8, u64::MAX - 31),
        ] {
            let mut bytes = header(1, None);
            tensor(&mut bytes, "weights", &shape, kind, offset);
            assert!(read(Cursor::new(finish(bytes, 256))).is_err());
        }
        for (second_name, second_offset) in [("different", 0), ("same", 32)] {
            let mut bytes = header(2, None);
            tensor(&mut bytes, "same", &[16], 0, 0);
            tensor(&mut bytes, second_name, &[1], 0, second_offset);
            assert!(read(Cursor::new(finish(bytes, 128))).is_err());
        }
    }
    #[test]
    fn rejects_duplicate_keys_nested_arrays_unknown_types_and_huge_strings() {
        let nested = [
            9_u32.to_le_bytes().as_slice(),
            1_u64.to_le_bytes().as_slice(),
        ]
        .concat();
        let huge = u64::MAX.to_le_bytes().to_vec();
        let mut duplicate = Vec::new();
        string(&mut duplicate, "qwen3");
        for extra in [
            ("general.architecture", 8, duplicate),
            ("array", 9, nested),
            ("unknown", 99, vec![]),
            ("huge_string", 8, huge),
            ("general.alignment", 4, 3_u32.to_le_bytes().to_vec()),
            ("general.alignment", 4, 8192_u32.to_le_bytes().to_vec()),
        ] {
            let mut bytes = header(1, Some(extra));
            tensor(&mut bytes, "weight", &[1], 0, 0);
            assert!(read(Cursor::new(finish(bytes, 4))).is_err());
        }
        let mut non_utf8 = 1_u64.to_le_bytes().to_vec();
        non_utf8.push(0xff);
        assert!(
            Input::new(Cursor::new(non_utf8))
                .unwrap()
                .string(16)
                .is_err()
        );
    }
}
