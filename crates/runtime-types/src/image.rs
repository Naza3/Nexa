//! Bounded, owned image input for the private worker protocol.
use crate::RuntimeError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;
pub const MAX_IMAGE_DIMENSION: u32 = 8192;
const MAX_ENCODED_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInput {
    pub mime_type: String,
    pub data: String,
    /// Preserve the two-part user message order across the worker boundary.
    #[serde(default)]
    pub after_text: bool,
}

impl fmt::Debug for ImageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageInput")
            .field("mime_type", &self.mime_type)
            .field("encoded_bytes", &self.data.len())
            .field("after_text", &self.after_text)
            .finish()
    }
}

impl ImageInput {
    pub fn from_data_url(url: &str) -> Result<Self, RuntimeError> {
        let (mime_type, data) = if let Some(data) = url.strip_prefix("data:image/png;base64,") {
            ("image/png", data)
        } else if let Some(data) = url.strip_prefix("data:image/jpeg;base64,") {
            ("image/jpeg", data)
        } else {
            return Err(invalid("OCR accepts only inline PNG or JPEG data URLs"));
        };
        if data.len() > MAX_ENCODED_BYTES {
            return Err(invalid("image exceeds the 4 MiB file limit"));
        }
        let image = Self {
            mime_type: mime_type.into(),
            data: data.into(),
            after_text: false,
        };
        image.decode()?;
        Ok(image)
    }

    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.data)
    }

    /// Validates format headers and allocation bounds. The native decoder must
    /// additionally validate the complete image before using any pixels.
    pub fn decode(&self) -> Result<Vec<u8>, RuntimeError> {
        if self.data.is_empty() || self.data.len() > MAX_ENCODED_BYTES {
            return Err(invalid("image exceeds the 4 MiB file limit or is empty"));
        }
        let bytes = STANDARD
            .decode(&self.data)
            .map_err(|_| invalid("invalid image base64"))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(invalid("image exceeds the 4 MiB file limit"));
        }
        let dimensions = match self.mime_type.as_str() {
            "image/png"
                if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    && bytes.len() >= 33
                    && bytes[8..12] == [0, 0, 0, 13]
                    && &bytes[12..16] == b"IHDR" =>
            {
                Some((
                    u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
                    u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                ))
            }
            "image/jpeg" => jpeg_dimensions(&bytes),
            _ => None,
        }
        .ok_or_else(|| invalid("image MIME and PNG/JPEG header do not match"))?;
        let (width, height) = dimensions;
        if width == 0
            || height == 0
            || width > MAX_IMAGE_DIMENSION
            || height > MAX_IMAGE_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
        {
            return Err(invalid(
                "image exceeds 8192 pixels per side or 16 megapixels",
            ));
        }
        Ok(bytes)
    }
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::invalid(message)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut offset = 2usize;
    while offset < bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 || marker == 0 {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
        if length < 2 || offset.checked_add(length)? > bytes.len() {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 {
                return None;
            }
            let height = u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?);
            let width = u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?);
            return Some((width.into(), height.into()));
        }
        offset += length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(width: u32, height: u32) -> ImageInput {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        ImageInput {
            mime_type: "image/png".into(),
            data: STANDARD.encode(bytes),
            after_text: false,
        }
    }
    #[test]
    fn bounded_png_and_data_url_roundtrip() {
        let image = png(1600, 1200);
        assert_eq!(ImageInput::from_data_url(&image.data_url()).unwrap(), image);
        assert!(image.decode().is_ok());
        assert!(!format!("{image:?}").contains(&image.data));
    }
    #[test]
    fn reject_dimensions_mime_remote_and_bad_base64() {
        for image in [png(0, 1), png(8193, 1), png(8192, 8192)] {
            assert!(image.decode().is_err());
        }
        let mut image = png(20, 20);
        image.mime_type = "image/jpeg".into();
        assert!(image.decode().is_err());
        assert!(ImageInput::from_data_url("https://example.com/image.png").is_err());
        assert!(ImageInput::from_data_url("data:image/png;base64,invalid!").is_err());
        assert!(
            ImageInput {
                mime_type: "image/png".into(),
                data: "A".repeat(MAX_ENCODED_BYTES + 1),
                after_text: false
            }
            .decode()
            .is_err()
        );
    }
    #[test]
    fn jpeg_dimensions_require_bounded_segments() {
        let bytes = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 10, 0, 20, 1, 1, 0x11, 0,
        ];
        assert_eq!(jpeg_dimensions(&bytes), Some((20, 10)));
        assert_eq!(jpeg_dimensions(&bytes[..14]), None);
    }
}
