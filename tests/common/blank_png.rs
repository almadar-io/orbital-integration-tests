//! Blank-render gate shared by the iOS and Android native smoke tests (the
//! Android test includes this file via `#[path]`). Dependency-free on purpose:
//! neither shell crate carries a PNG decoder in its dev-dependencies, so this
//! decodes just enough of PNG (stored/fixed/dynamic DEFLATE, all five scanline
//! filters, non-interlaced, bit depth 8 or 16) to answer "is every pixel the
//! same colour?".
#![allow(dead_code)]

use std::path::Path;

/// md5 of the image swift-snapshot-testing writes in place of a snapshot when a
/// view rendered at zero size ("View has zero size" error banner, 1200x240).
/// It is a rendered text image, so the only deterministic identity it has is
/// its exact bytes; if a toolchain bump changes the banner the uniform-colour
/// check still cannot see it (the banner has text), and this constant must be
/// refreshed from any `size was zero` failure.
pub const IOS_ZERO_SIZE_ERROR_IMAGE_MD5: &str = "c3139eb047f928de78f60f25616d6811";

#[derive(Debug, PartialEq, Eq)]
pub enum PngVerdict {
    Varied,
    /// Every pixel is identical; carries `(width, height)`.
    Uniform(u32, u32),
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl<'a> Bits<'a> {
    fn read(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self.data.get(self.pos).ok_or("deflate stream truncated")?;
            v |= (((byte >> self.bit) & 1) as u32) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(v)
    }
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.read(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("invalid huffman code".into())
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut bits = Bits {
        data,
        pos: 0,
        bit: 0,
    };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = bits.read(1)?;
        match bits.read(2)? {
            0 => {
                bits.align();
                let p = bits.pos;
                let hdr = data.get(p..p + 4).ok_or("stored block truncated")?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                let body = data
                    .get(p + 4..p + 4 + len)
                    .ok_or("stored block truncated")?;
                out.extend_from_slice(body);
                bits.pos = p + 4 + len;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    let mut l = [0u8; 288];
                    for (i, v) in l.iter_mut().enumerate() {
                        *v = match i {
                            0..=143 => 8,
                            144..=255 => 9,
                            256..=279 => 7,
                            _ => 8,
                        };
                    }
                    (Huffman::new(&l), Huffman::new(&[5u8; 30]))
                } else {
                    let hlit = bits.read(5)? as usize + 257;
                    let hdist = bits.read(5)? as usize + 1;
                    let hclen = bits.read(4)? as usize + 4;
                    const ORDER: [usize; 19] = [
                        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                    ];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(hclen) {
                        cl[o] = bits.read(3)? as u8;
                    }
                    let clh = Huffman::new(&cl);
                    let mut lengths = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < hlit + hdist {
                        let sym = clh.decode(&mut bits)?;
                        match sym {
                            0..=15 => {
                                lengths[i] = sym as u8;
                                i += 1;
                            }
                            16 => {
                                let prev = *lengths
                                    .get(i.wrapping_sub(1))
                                    .ok_or("repeat with no previous length")?;
                                let n = 3 + bits.read(2)? as usize;
                                for _ in 0..n {
                                    *lengths.get_mut(i).ok_or("length repeat overflow")? = prev;
                                    i += 1;
                                }
                            }
                            _ => {
                                let n = if sym == 17 {
                                    3 + bits.read(3)?
                                } else {
                                    11 + bits.read(7)?
                                } as usize;
                                for _ in 0..n {
                                    *lengths.get_mut(i).ok_or("length repeat overflow")? = 0;
                                    i += 1;
                                }
                            }
                        }
                    }
                    (
                        Huffman::new(&lengths[..hlit]),
                        Huffman::new(&lengths[hlit..]),
                    )
                };
                loop {
                    let sym = lit.decode(&mut bits)? as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let s = sym - 257;
                        if s >= 29 {
                            return Err("invalid length symbol".into());
                        }
                        let len = LEN_BASE[s] as usize + bits.read(LEN_EXTRA[s] as u32)? as usize;
                        let ds = dist.decode(&mut bits)? as usize;
                        if ds >= 30 {
                            return Err("invalid distance symbol".into());
                        }
                        let d = DIST_BASE[ds] as usize + bits.read(DIST_EXTRA[ds] as u32)? as usize;
                        if d > out.len() {
                            return Err("distance beyond output".into());
                        }
                        for _ in 0..len {
                            out.push(out[out.len() - d]);
                        }
                    }
                }
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

/// Decode `png` far enough to say whether every pixel is the same colour.
pub fn classify_png(png: &[u8]) -> Result<PngVerdict, String> {
    if png.get(..8) != Some(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Err("not a PNG".into());
    }
    let (mut pos, mut hdr, mut idat) = (8usize, None, Vec::new());
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let kind = &png[pos + 4..pos + 8];
        let body = png.get(pos + 8..pos + 8 + len).ok_or("chunk truncated")?;
        match kind {
            b"IHDR" => hdr = Some(body.to_vec()),
            b"IDAT" => idat.extend_from_slice(body),
            _ => {}
        }
        pos += 12 + len;
    }
    let h = hdr.ok_or("missing IHDR")?;
    let w = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
    let ht = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
    let (depth, color, interlace) = (h[8], h[9], h[12]);
    if interlace != 0 {
        return Err("interlaced PNG unsupported".into());
    }
    if depth != 8 && depth != 16 {
        return Err(format!("bit depth {depth} unsupported"));
    }
    let channels: usize = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        c => return Err(format!("colour type {c} unsupported")),
    };
    if w == 0 || ht == 0 {
        return Err("zero-size PNG".into());
    }
    let bpp = channels * (depth as usize / 8);
    let stride = w as usize * bpp;
    let raw = inflate(idat.get(2..).ok_or("zlib stream truncated")?)?;
    if raw.len() < (stride + 1) * ht as usize {
        return Err("pixel data shorter than IHDR claims".into());
    }
    let mut prev = vec![0u8; stride];
    let mut cur = vec![0u8; stride];
    let mut uniform = true;
    let mut first: Option<Vec<u8>> = None;
    for row in 0..ht as usize {
        let line = &raw[row * (stride + 1)..(row + 1) * (stride + 1)];
        let filter = line[0];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let x = line[i + 1] as i32;
            let v = match filter {
                0 => x,
                1 => x + a,
                2 => x + b,
                3 => x + (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    x + if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                f => return Err(format!("invalid filter {f}")),
            };
            cur[i] = v as u8;
        }
        let reference = first.get_or_insert_with(|| cur[..bpp].to_vec());
        if cur.chunks(bpp).any(|px| px != reference.as_slice()) {
            uniform = false;
            break;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(if uniform {
        PngVerdict::Uniform(w, ht)
    } else {
        PngVerdict::Varied
    })
}

/// Patterns whose render is uniform by contract, each with the reason. They
/// still must decode and must not be the zero-size error image; only the
/// uniform-colour verdict is waived. Add a pattern here only when painting
/// nothing (or one flat colour) IS its specified output.
pub const INTENTIONALLY_UNIFORM: &[(&str, &str)] = &[
    (
        "infinite-scroll-sentinel",
        "invisible sentinel row that only triggers a load-more event",
    ),
    (
        "overlay",
        "full-bleed scrim: a single flat colour when it has no children",
    ),
    ("spacer", "pure whitespace, paints nothing"),
    (
        "game-audio-cue",
        "audio-only behavior atom: the web GameAudioCue returns null",
    ),
];

fn is_intentionally_uniform(pattern: &str) -> bool {
    INTENTIONALLY_UNIFORM
        .iter()
        .any(|(name, _)| name.replace('-', "_") == pattern.replace('-', "_"))
}

/// One failure line per PNG in `files` that is a decode error, a uniform
/// colour (unless intentionally so), or (when `zero_size_md5` is set) the iOS
/// zero-size error image. `name_of` maps a file to the pattern name.
pub fn blank_failures(
    files: &[std::path::PathBuf],
    zero_size_md5: Option<&str>,
    name_of: impl Fn(&Path) -> String,
) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();
    for f in files {
        let name = name_of(f);
        let bytes = std::fs::read(f).unwrap_or_else(|e| panic!("read {}: {e}", f.display()));
        if let Some(md5) = zero_size_md5 {
            if md5_hex(&bytes) == md5 {
                failures.push(format!(
                    "{name}: rendered at zero size (swift-snapshot-testing error image)"
                ));
                continue;
            }
        }
        match classify_png(&bytes) {
            Ok(PngVerdict::Varied) => {}
            Ok(PngVerdict::Uniform(..)) if is_intentionally_uniform(&name) => {}
            Ok(PngVerdict::Uniform(w, h)) => failures.push(format!(
                "{name}: blank render, every pixel identical ({w}x{h})"
            )),
            Err(e) => failures.push(format!("{name}: undecodable PNG ({e})")),
        }
    }
    failures.sort();
    failures
}

/// Drop failures for patterns named in `exempt` (comma-separated, `-`/`_`
/// interchangeable). Pure so the filter is unit-testable.
/// Panic naming every pattern `blank_failures` reports.
pub fn assert_no_blank_renders(
    files: &[std::path::PathBuf],
    zero_size_md5: Option<&str>,
    name_of: impl Fn(&Path) -> String,
) {
    let failures = blank_failures(files, zero_size_md5, name_of);
    assert!(
        failures.is_empty(),
        "{} snapshot(s) are not real renders:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// md5 via the system `md5` tool is avoided (portability); small in-file MD5.
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc_free_png(w: u32, h: u32, color: u8, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut raw = Vec::new();
        for r in rows {
            raw.push(r[0]);
            raw.extend_from_slice(&r[1..]);
        }
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(&raw);
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, color, 0, 0, 0]);
        for (kind, body) in [(&b"IHDR"[..], ihdr), (&b"IDAT"[..], z)] {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(&body);
            out.extend_from_slice(&[0, 0, 0, 0]);
        }
        out
    }

    #[test]
    fn uniform_rgba_is_blank() {
        let row = vec![vec![0u8, 10, 20, 30, 255, 10, 20, 30, 255]; 3];
        assert_eq!(
            classify_png(&crc_free_png(2, 3, 6, &row)).unwrap(),
            PngVerdict::Uniform(2, 3)
        );
    }

    #[test]
    fn one_different_pixel_is_varied() {
        let rows = [
            vec![0u8, 10, 20, 30, 255, 10, 20, 30, 255],
            vec![0, 10, 20, 30, 255, 10, 20, 31, 255],
        ];
        assert_eq!(
            classify_png(&crc_free_png(2, 2, 6, &rows)).unwrap(),
            PngVerdict::Varied
        );
    }

    #[test]
    fn sub_filter_is_undone_before_comparing() {
        // Sub-filtered row [p, 0] decodes to two identical pixels.
        let rows = [vec![1u8, 10, 20, 30, 255, 0, 0, 0, 0]];
        assert_eq!(
            classify_png(&crc_free_png(2, 1, 6, &rows)).unwrap(),
            PngVerdict::Uniform(2, 1)
        );
        // Sub-filtered row [p, +1 on blue] decodes to two different pixels.
        let rows = [vec![1u8, 10, 20, 30, 255, 0, 0, 1, 0]];
        assert_eq!(
            classify_png(&crc_free_png(2, 1, 6, &rows)).unwrap(),
            PngVerdict::Varied
        );
    }

    fn write_png(name: &str, w: u32, rows: &[Vec<u8>]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("blank_png_tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, crc_free_png(w, rows.len() as u32, 6, rows)).unwrap();
        path
    }

    fn stem(p: &Path) -> String {
        p.file_stem().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn gate_names_blank_pattern_and_waives_intentionally_uniform() {
        let flat = vec![vec![0u8, 1, 2, 3, 255, 1, 2, 3, 255]];
        let varied = vec![vec![0u8, 1, 2, 3, 255, 9, 9, 9, 255]];
        let files = [
            write_png("accordion.png", 2, &flat),
            write_png("spacer.png", 2, &flat),
            write_png("badge.png", 2, &varied),
        ];
        let failures = blank_failures(&files, None, stem);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].starts_with("accordion: blank render"),
            "{failures:?}"
        );
    }

    #[test]
    fn audio_only_atom_is_intentionally_uniform_but_a_visual_atom_is_not() {
        assert!(is_intentionally_uniform("game_audio_cue"));
        assert!(is_intentionally_uniform("game-audio-cue"));
        assert!(!is_intentionally_uniform("game_hud"));
    }

    #[test]
    fn gate_flags_the_zero_size_error_image_by_md5() {
        let path = write_png("carousel.png", 2, &[vec![0u8, 1, 2, 3, 255, 9, 9, 9, 255]]);
        let md5 = md5_hex(&std::fs::read(&path).unwrap());
        let failures = blank_failures(std::slice::from_ref(&path), Some(&md5), stem);
        assert!(
            failures[0].starts_with("carousel: rendered at zero size"),
            "{failures:?}"
        );
        assert!(blank_failures(
            std::slice::from_ref(&path),
            Some(IOS_ZERO_SIZE_ERROR_IMAGE_MD5),
            stem
        )
        .is_empty());
    }

    #[test]
    fn garbage_is_an_error_not_a_pass() {
        assert!(classify_png(b"not a png").is_err());
    }

    #[test]
    fn md5_known_vector() {
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn every_checked_in_snapshot_decodes() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-schemas/native-conformance/snapshots");
        let mut decoded = 0;
        for platform in ["ios", "android"] {
            let Ok(dir) = std::fs::read_dir(root.join(platform)) else {
                continue;
            };
            for entry in dir.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("png") {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                classify_png(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                decoded += 1;
            }
        }
        assert!(
            decoded > 0 || !root.exists(),
            "snapshot dir exists but no PNG decoded"
        );
    }
}
