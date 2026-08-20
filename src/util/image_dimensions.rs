//! Dependency-free PNG/JPEG dimension reader. Reads just the header bytes
//! already in hand right after a download (`ensure_page_downloaded` has the
//! full `Vec<u8>` in memory anyway) - no image-decoding crate needed, since
//! width/height sit at fixed, well-documented byte offsets in both formats.
//!
//! Returns `None` for anything else (webp/gif/avif/bmp) or malformed/
//! truncated bytes - callers degrade gracefully (the reader falls back to a
//! fixed-size placeholder for those pages, same as before this existed).

pub fn read_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    read_png_dimensions(bytes).or_else(|| read_jpeg_dimensions(bytes))
}

fn read_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

    if bytes.len() < 24 || &bytes[0..8] != PNG_SIGNATURE {
        return None;
    }
    // IHDR is always the first chunk: 4-byte length, then "IHDR", then
    // width/height as big-endian u32s.
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn read_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            // Not a marker where one was expected - bail rather than guess.
            return None;
        }
        let marker = bytes[i + 1];

        // Standalone markers with no length/payload.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }

        let segment_length = u16::from_be_bytes(bytes.get(i + 2..i + 4)?.try_into().ok()?) as usize;
        // SOF0-SOF15 mark a frame header, except DHT (C4), JPG (C8, reserved/unused),
        // and DAC (CC), which share the same numeric range but aren't SOF markers.
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);

        if is_sof {
            let payload = bytes.get(i + 4..i + 4 + 5)?;
            let height = u16::from_be_bytes(payload[1..3].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(payload[3..5].try_into().ok()?) as u32;
            return Some((width, height));
        }

        i += 2 + segment_length;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_dimensions() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec(); // signature
        bytes.extend_from_slice(&[0, 0, 0, 13]); // chunk length (unused by our reader)
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&100u32.to_be_bytes()); // width
        bytes.extend_from_slice(&200u32.to_be_bytes()); // height
        assert_eq!(read_dimensions(&bytes), Some((100, 200)));
    }

    #[test]
    fn jpeg_dimensions_sof0() {
        let mut bytes = vec![0xFF, 0xD8]; // SOI
        bytes.extend_from_slice(&[0xFF, 0xC0]); // SOF0
        bytes.extend_from_slice(&17u16.to_be_bytes()); // segment length
        bytes.push(8); // precision
        bytes.extend_from_slice(&400u16.to_be_bytes()); // height
        bytes.extend_from_slice(&300u16.to_be_bytes()); // width
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]); // component data (unread)
        assert_eq!(read_dimensions(&bytes), Some((300, 400)));
    }

    #[test]
    fn jpeg_skips_preceding_segments() {
        let mut bytes = vec![0xFF, 0xD8]; // SOI
                                          // A APP0/JFIF segment before the real SOF - reader must skip over it correctly.
        bytes.extend_from_slice(&[0xFF, 0xE0]);
        bytes.extend_from_slice(&16u16.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 14]);
        bytes.extend_from_slice(&[0xFF, 0xC2]); // SOF2 (progressive)
        bytes.extend_from_slice(&17u16.to_be_bytes());
        bytes.push(8);
        bytes.extend_from_slice(&50u16.to_be_bytes()); // height
        bytes.extend_from_slice(&60u16.to_be_bytes()); // width
        bytes.extend_from_slice(&[0u8; 10]);
        assert_eq!(read_dimensions(&bytes), Some((60, 50)));
    }

    #[test]
    fn unsupported_format_returns_none() {
        assert_eq!(read_dimensions(b"RIFF....WEBPVP8 "), None);
        assert_eq!(read_dimensions(b"not an image"), None);
        assert_eq!(read_dimensions(&[]), None);
    }
}
