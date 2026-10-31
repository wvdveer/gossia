//! ClioTree's icon, drawn in code so that no image file is kept in the
//! repository: a blue rounded square with a white tree, the outline of a
//! root and two children. The window uses it, and the build script puts it
//! in the Windows executable (`build.rs`), which is why this file uses
//! nothing but the standard library.

/// The blue of the square: the accent of the light palette.
const BLUE: [u8; 3] = [0x24, 0x7B, 0xC1];

/// The pixels of the icon at `size` by `size`, as RGBA rows from the top.
pub fn pixels(size: u32) -> Vec<u8> {
    // Each pixel is the share of 4 by 4 samples inside each shape.
    const N: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut square, mut white) = (0u32, 0u32);
            for sy in 0..N {
                for sx in 0..N {
                    let u = (x as f32 + (sx as f32 + 0.5) / N as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / N as f32) / size as f32;
                    if in_square(u, v) {
                        square += 1;
                        if in_tree(u, v) {
                            white += 1;
                        }
                    }
                }
            }
            let total = (N * N) as f32;
            let alpha = square as f32 / total;
            let share = if square == 0 {
                0.0
            } else {
                white as f32 / square as f32
            };
            let mix = |c: u8| (c as f32 + (255.0 - c as f32) * share).round() as u8;
            out.extend([
                mix(BLUE[0]),
                mix(BLUE[1]),
                mix(BLUE[2]),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    out
}

/// Inside the rounded square that fills the icon, with a small margin.
fn in_square(u: f32, v: f32) -> bool {
    rounded(u, v, (0.03, 0.03, 0.97, 0.97), 0.2)
}

/// Inside the white tree: a root node with its label, a trunk going down
/// from it, and two branches, each to the label of a child.
fn in_tree(u: f32, v: f32) -> bool {
    let line = 0.055;
    let root = (u - 0.30).powi(2) + (v - 0.29).powi(2) <= 0.09f32.powi(2);
    let root_label = rounded(u, v, (0.45, 0.24, 0.80, 0.34), 0.05);
    let trunk = rounded(
        u,
        v,
        (
            0.30 - line / 2.0,
            0.29,
            0.30 + line / 2.0,
            0.71 + line / 2.0,
        ),
        0.0,
    );
    let branch = |y: f32| rounded(u, v, (0.30, y - line / 2.0, 0.50, y + line / 2.0), 0.0);
    let label = |y: f32| rounded(u, v, (0.47, y - 0.05, 0.80, y + 0.05), 0.05);
    root || root_label || trunk || branch(0.50) || branch(0.71) || label(0.50) || label(0.71)
}

/// Inside the rectangle `(left, top, right, bottom)` with corners of radius `r`.
fn rounded(u: f32, v: f32, (l, t, r_, b): (f32, f32, f32, f32), r: f32) -> bool {
    if u < l || u > r_ || v < t || v > b {
        return false;
    }
    let cx = u.clamp(l + r, r_ - r);
    let cy = v.clamp(t + r, b - r);
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

/// The icon as the resources of a Windows executable (a `.res` file, which
/// the linker takes as it is): the icon in several sizes, and the version
/// details Explorer shows (`version` is `major.minor.patch`).
pub fn windows_resources(version: &str) -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    const RT_VERSION: u16 = 16;
    let sizes = [16u32, 24, 32, 48, 64, 256];
    let mut res = Vec::new();
    // A .res file starts with an empty entry.
    resource(&mut res, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((sizes.len() as u16).to_le_bytes());
    for (i, &size) in sizes.iter().enumerate() {
        let image = dib(size);
        let id = i as u16 + 1;
        // 256 is written as 0.
        group.push(size as u8);
        group.push(size as u8);
        group.extend([0u8, 0]);
        group.extend(1u16.to_le_bytes());
        group.extend(32u16.to_le_bytes());
        group.extend((image.len() as u32).to_le_bytes());
        group.extend(id.to_le_bytes());
        resource(&mut res, RT_ICON, id, &image);
    }
    resource(&mut res, RT_GROUP_ICON, 1, &group);
    resource(&mut res, RT_VERSION, 1, &version_info(version));
    res
}

/// The icon at one size as an icon resource holds it: a 32-bit bitmap,
/// bottom row first, of twice the height (the mask, all shown, follows it).
fn dib(size: u32) -> Vec<u8> {
    let rgba = pixels(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40u32, size, size * 2] {
        out.extend(v.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((size * size * 4 + mask_row * size).to_le_bytes());
    out.extend([0u8; 16]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}

/// One entry of a .res file, with a numeric type and name.
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(kind.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(name.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    // Moveable, pure and discardable; US English.
    out.extend(0x1030u16.to_le_bytes());
    out.extend(0x0409u16.to_le_bytes());
    out.extend([0u8; 8]);
    out.extend(data);
    pad(out);
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A block of a version resource: its key, its value (text is counted in
/// characters, binary in bytes) and the blocks inside it.
fn block(key: &str, text: bool, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let value_length = if text { value.len() / 2 } else { value.len() };
    let mut out = vec![0, 0];
    out.extend((value_length as u16).to_le_bytes());
    out.extend((text as u16).to_le_bytes());
    out.extend(utf16(key));
    pad(&mut out);
    out.extend(value);
    for child in children {
        pad(&mut out);
        out.extend(child);
    }
    let length = (out.len() as u16).to_le_bytes();
    out[..2].copy_from_slice(&length);
    out
}

/// The version resource: the numbers, and the texts Explorer shows.
fn version_info(version: &str) -> Vec<u8> {
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0) as u32,
        parts.next().unwrap_or(0) as u32,
        parts.next().unwrap_or(0) as u32,
    );
    let ms = (major << 16) | minor;
    let ls = patch << 16;
    let mut fixed = Vec::new();
    for v in [
        0xFEEF04BDu32, // the signature
        0x0001_0000,
        ms,
        ls,
        ms,
        ls,
        0x3F,
        0,
        0x0004_0004, // Windows NT
        1,           // an application
        0,
        0,
        0,
    ] {
        fixed.extend(v.to_le_bytes());
    }
    let texts = [
        ("CompanyName", "ClioTree"),
        ("FileDescription", "ClioTree"),
        ("FileVersion", version),
        ("InternalName", "cliotree"),
        ("OriginalFilename", "cliotree.exe"),
        ("ProductName", "ClioTree"),
        ("ProductVersion", version),
    ];
    let strings: Vec<Vec<u8>> = texts
        .iter()
        .map(|(key, value)| block(key, true, &utf16(value), &[]))
        .collect();
    // US English, Unicode.
    let table = block("040904B0", true, &[], &strings);
    let string_info = block("StringFileInfo", true, &[], &[table]);
    let mut translation = Vec::new();
    translation.extend(0x0409u16.to_le_bytes());
    translation.extend(1200u16.to_le_bytes());
    let var = block("Translation", false, &translation, &[]);
    let var_info = block("VarFileInfo", true, &[], &[var]);
    block("VS_VERSION_INFO", false, &fixed, &[string_info, var_info])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_a_blue_square_a_white_tree_and_clear_corners() {
        let size = 64;
        let px = pixels(size);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "the corner is clear");
        assert_eq!(at(8, 58), [BLUE[0], BLUE[1], BLUE[2], 255]);
        // The root node, white.
        assert_eq!(at(19, 18), [255, 255, 255, 255]);
    }

    #[test]
    fn the_resources_hold_the_icons_and_the_version() {
        let res = windows_resources("1.2.3");
        // The empty entry first, then entries aligned to four bytes.
        assert_eq!(&res[..8], &[0, 0, 0, 0, 32, 0, 0, 0]);
        assert!(res.len().is_multiple_of(4));
        let text: Vec<u16> = res
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_le_bytes(b))
            .collect();
        let has = |s: &str| {
            let needle: Vec<u16> = s.encode_utf16().collect();
            text.windows(needle.len()).any(|w| w == needle)
        };
        assert!(has("VS_VERSION_INFO") && has("1.2.3") && has("cliotree.exe"));
    }
}
