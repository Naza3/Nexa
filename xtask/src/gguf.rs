//! Bounded GGUF metadata inspection. This is not a model loader or inference test.
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

use crate::Result;

const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STRING_BYTES: u64 = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Metadata {
    pub architecture: String,
    pub file_type: u32,
    pub template: String,
    pub context_length: u32,
}

pub fn read<R: Read + Seek>(reader: R) -> Result<Metadata> {
    let mut input = Input::new(reader)?;
    if input.bytes::<4>()? != *b"GGUF" {
        return Err("invalid GGUF magic".into());
    }
    if !matches!(input.u32()?, 2 | 3) {
        return Err("unsupported GGUF version (expected 2 or 3)".into());
    }
    if input.u64()? == 0 {
        return Err("GGUF has no tensors".into());
    }
    let count = input.u64()?;
    if count > 100_000 {
        return Err("GGUF metadata count exceeds verification limit".into());
    }
    let mut strings = BTreeMap::new();
    let mut numbers = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..count {
        let key = input.string(4096)?;
        if !seen.insert(key.clone()) {
            return Err("duplicate GGUF metadata key".into());
        }
        let kind = input.u32()?;
        match key.as_str() {
            "general.architecture" | "tokenizer.chat_template" => {
                if kind != 8 {
                    return Err(format!("GGUF {key} must be a string").into());
                }
                strings.insert(key, input.string(MAX_STRING_BYTES)?);
            }
            "general.file_type" => {
                if kind != 4 {
                    return Err("GGUF general.file_type must be uint32".into());
                }
                numbers.insert(key, input.u32()?);
            }
            _ if key.ends_with(".context_length") => {
                if kind != 4 {
                    return Err("GGUF context_length must be uint32".into());
                }
                numbers.insert(key, input.u32()?);
            }
            _ => input.skip_value(kind)?,
        }
    }
    let architecture = strings
        .remove("general.architecture")
        .filter(|s| !s.is_empty())
        .ok_or("GGUF architecture missing")?;
    Ok(Metadata {
        context_length: *numbers
            .get(&format!("{architecture}.context_length"))
            .ok_or("GGUF context length missing")?,
        architecture,
        file_type: *numbers
            .get("general.file_type")
            .ok_or("GGUF file type missing")?,
        template: strings
            .remove("tokenizer.chat_template")
            .filter(|s| !s.is_empty())
            .ok_or("GGUF chat template missing")?,
    })
}

struct Input<R> {
    reader: R,
    position: u64,
    limit: u64,
}

impl<R: Read + Seek> Input<R> {
    fn new(mut reader: R) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader,
            position: 0,
            limit: length.min(MAX_METADATA_BYTES),
        })
    }
    fn reserve(&mut self, length: u64) -> Result<()> {
        self.position = self
            .position
            .checked_add(length)
            .filter(|&n| n <= self.limit)
            .ok_or("truncated or oversized GGUF metadata")?;
        Ok(())
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.reserve(N as u64)?;
        let mut bytes = [0; N];
        self.reader.read_exact(&mut bytes)?;
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
            return Err("GGUF string exceeds verification limit".into());
        }
        self.reserve(length)?;
        let mut bytes = vec![0; length as usize];
        self.reader.read_exact(&mut bytes)?;
        Ok(String::from_utf8(bytes)?)
    }
    fn skip(&mut self, length: u64) -> Result<()> {
        self.reserve(length)?;
        self.reader.seek(SeekFrom::Start(self.position))?;
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
                    return Err("unsupported or oversized GGUF array".into());
                }
                if element == 8 {
                    for _ in 0..count {
                        self.skip_value(element)?;
                    }
                    Ok(())
                } else {
                    let size = scalar_size(element)?;
                    self.skip(count.checked_mul(size).ok_or("GGUF array overflow")?)
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
        _ => Err("unknown GGUF metadata type".into()),
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
    fn fixture() -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(1_u64.to_le_bytes());
        bytes.extend(4_u64.to_le_bytes());
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
        bytes
    }
    #[test]
    fn reads_exact_template_bytes() {
        assert_eq!(
            read(Cursor::new(fixture())).unwrap(),
            Metadata {
                architecture: "qwen3".into(),
                file_type: 7,
                template: "template\n中文".into(),
                context_length: 40960,
            }
        );
    }
    #[test]
    fn rejects_all_truncations() {
        let bytes = fixture();
        for end in 0..bytes.len() {
            assert!(read(Cursor::new(&bytes[..end])).is_err(), "end={end}");
        }
    }
    #[test]
    fn rejects_invalid_magic_version_and_oversized_count() {
        for (offset, replacement) in [(0, 0_u64), (4, 9), (16, u64::MAX)] {
            let mut bytes = fixture();
            bytes[offset..offset + 8].copy_from_slice(&replacement.to_le_bytes());
            assert!(read(Cursor::new(bytes)).is_err());
        }
    }
    #[test]
    fn rejects_duplicate_metadata_and_non_utf8_strings() {
        let mut bytes = fixture();
        bytes[16..24].copy_from_slice(&5_u64.to_le_bytes());
        string(&mut bytes, "general.architecture");
        bytes.extend(8_u32.to_le_bytes());
        string(&mut bytes, "qwen3");
        assert!(read(Cursor::new(bytes)).is_err());
        let mut bad_string = 1_u64.to_le_bytes().to_vec();
        bad_string.push(0xff);
        assert!(
            Input::new(Cursor::new(bad_string))
                .unwrap()
                .string(16)
                .is_err()
        );
    }
    #[test]
    fn refuses_nested_array_and_unknown_scalar() {
        let mut bytes = 9_u32.to_le_bytes().to_vec();
        bytes.extend(1_u64.to_le_bytes());
        assert!(
            Input::new(Cursor::new(bytes))
                .unwrap()
                .skip_value(9)
                .is_err()
        );
        assert!(scalar_size(13).is_err());
    }
}
