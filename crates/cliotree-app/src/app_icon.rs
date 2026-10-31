//! ClioTree's icon, drawn in code so that no image file is kept in the
//! repository: the "bracket canopy" (from the Canva design thread), a tree
//! whose crown is a pair of green JSON braces on a white trunk with a flared
//! base, on a near-black rounded square with a thin white outline. The window
//! uses it, and the build script puts it in the Windows executable
//! (`build.rs`), which is why this file uses nothing but the standard library.

/// The square behind the tree.
const BACK: [u8; 3] = [0x14, 0x17, 0x1C];
/// The braces of the crown.
const GREEN: [u8; 3] = [0x7C, 0xCC, 0x7E];
/// The trunk and the outline.
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// The pixels of the icon at `size` by `size`, as RGBA rows from the top.
pub fn pixels(size: u32) -> Vec<u8> {
    // Each pixel mixes the colours of 4 by 4 samples.
    const N: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut covered, mut sum) = (0u32, [0u32; 3]);
            for sy in 0..N {
                for sx in 0..N {
                    let u = (x as f32 + (sx as f32 + 0.5) / N as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / N as f32) / size as f32;
                    if let Some(c) = colour_at(u, v) {
                        covered += 1;
                        for (s, c) in sum.iter_mut().zip(c) {
                            *s += c as u32;
                        }
                    }
                }
            }
            let total = N * N;
            let channel = |s: u32| {
                if covered == 0 {
                    0
                } else {
                    (s as f32 / covered as f32).round() as u8
                }
            };
            out.extend([
                channel(sum[0]),
                channel(sum[1]),
                channel(sum[2]),
                (covered as f32 / total as f32 * 255.0).round() as u8,
            ]);
        }
    }
    out
}

/// The colour at a point of the icon (0 to 1 across and down), or `None`
/// outside the rounded square.
fn colour_at(u: f32, v: f32) -> Option<[u8; 3]> {
    if !rounded(u, v, (0.03, 0.03, 0.97, 0.97), 0.16) {
        return None;
    }
    let outline = rounded(u, v, (0.075, 0.075, 0.925, 0.925), 0.12)
        && !rounded(u, v, (0.093, 0.093, 0.907, 0.907), 0.102);
    Some(if in_braces(u, v) {
        GREEN
    } else if outline || in_trunk(u, v) {
        WHITE
    } else {
        BACK
    })
}

/// Inside the crown: a `{` and, mirrored, a `}`, each a bold stroke from
/// its top arm down past the point in the middle to a bottom arm that bends
/// in to the trunk.
fn in_braces(u: f32, v: f32) -> bool {
    // The left brace; a point right of the middle is taken as its mirror.
    let u = if u > 0.5 { 1.0 - u } else { u };
    let points = [
        (0.41, 0.16),
        (0.28, 0.16),
        (0.28, 0.32),
        (0.19, 0.385),
        (0.28, 0.45),
        (0.28, 0.54),
        (0.295, 0.575),
        (0.33, 0.60),
        (0.38, 0.612),
        (0.47, 0.615),
    ];
    points
        .windows(2)
        .any(|w| near_segment(u, v, w[0], w[1], 0.04))
}

/// Inside the trunk: narrow under the crown, flaring out to a flat base.
fn in_trunk(u: f32, v: f32) -> bool {
    let (top, base) = (0.60, 0.88);
    if !(top..=base).contains(&v) {
        return false;
    }
    let t = (v - top) / (base - top);
    let half = 0.045 + 0.19 * t.powi(3);
    (u - 0.5).abs() <= half
}

/// Within `r` of the segment from `a` to `b`.
fn near_segment(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r: f32) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((u - a.0) * dx + (v - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (px, py) = (a.0 + t * dx, a.1 + t * dy);
    (u - px).powi(2) + (v - py).powi(2) <= r * r
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
    fn the_icon_has_green_braces_a_white_trunk_and_clear_corners() {
        let size = 64;
        let px = pixels(size);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "the corner is clear");
        assert_eq!(at(10, 50), [BACK[0], BACK[1], BACK[2], 255]);
        // The point of the left brace, and its mirror.
        assert_eq!(at(13, 27), [GREEN[0], GREEN[1], GREEN[2], 255]);
        assert_eq!(at(50, 27), [GREEN[0], GREEN[1], GREEN[2], 255]);
        // The trunk.
        assert_eq!(at(32, 50), [255, 255, 255, 255]);
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
