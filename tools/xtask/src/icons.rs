//! `cargo xtask icons <dir>`: Gossia's icon as PNG files, for the packages
//! (the Linux icon theme and the MSIX logos). The icon is drawn by the same
//! code as the window's (`crates/gossia-app/src/app_icon.rs`), so the
//! repository keeps no image files.

use std::path::Path;

#[path = "../../../crates/gossia-app/src/app_icon.rs"]
#[allow(dead_code)]
mod app_icon;

/// The sizes written: the Linux icon theme's, and the MSIX logos' (44, 50
/// and 150).
pub const SIZES: &[u32] = &[16, 24, 32, 44, 48, 50, 64, 128, 150, 256, 512];

/// Writes `<size>.png` for each of `SIZES` into `dir`.
pub fn write(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for &size in SIZES {
        let file = dir.join(format!("{size}.png"));
        std::fs::write(&file, png(size, &app_icon::pixels(size)))
            .map_err(|e| format!("{}: {e}", file.display()))?;
    }
    println!("wrote {} icons to {}", SIZES.len(), dir.display());
    Ok(())
}

/// An RGBA image as a PNG file. The image data is zlib with stored
/// (uncompressed) blocks: the icons are small, and nothing more is needed
/// than the standard library.
pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(u8::from(blocks.peek().is_none()));
        let len = block.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend(size.to_be_bytes());
    header.extend(size.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, adaptive, no interlace

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_are_the_known_ones() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_png_has_its_signature_header_and_end() {
        let file = png(2, &[255; 16]);
        assert!(file.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&file[12..16], b"IHDR");
        assert_eq!(&file[16..24], &[0, 0, 0, 2, 0, 0, 0, 2]);
        assert!(file.ends_with(&[b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }
}
