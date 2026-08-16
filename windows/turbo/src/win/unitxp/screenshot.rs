//! Screenshot re-encoding: PNG or JPEG in place of the stock TGA write.
//!
//! The stock path formats a raw 24-bit TGA and writes it on the game thread,
//! a visible hitch at high resolutions. The wrapper takes the finished frame
//! buffer, copies it, and hands the copy to a short-lived worker thread that
//! swizzles BGR to RGB and encodes — the frame the player sees never waits
//! on the file. Anything but the plain 24-bit shape (or any setup failure)
//! runs the displaced writer unchanged, so a screenshot is never lost.
//!
//! Filenames keep the stock stem plus a `_NNN` counter so several shots can
//! land in the same second.

use core::sync::atomic::{AtomicU32, Ordering};
use std::path::PathBuf;

/// Bytes per pixel of the stock frame buffer (24-bit BGR).
const BYTES_PER_PIXEL: usize = 3;

/// JPEG quality, the encoder's 0-100 scale.
const JPEG_QUALITY: u8 = 95;

/// The `_NNN` filename counter, wrapping at 999.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The wrapper body; `true` when the shot was taken over.
///
/// `record` is the TGA writer object: header from `+0x8` (extra-header
/// length, color-map type, image type; dimensions at `+0xc`), pixel data
/// pointer at `+0x4`.
pub fn take_over(record: usize, tga_filename: *const u8) -> bool {
    if record == 0 || tga_filename.is_null() {
        return false;
    }
    // SAFETY: `record` is the live writer object the client dispatched with;
    // `+0x8` opens its embedded TGA header.
    let extra_header = unsafe { *((record + 0x8) as *const u8) };
    // SAFETY: `+0x9` is the header's color-map type.
    let color_map = unsafe { *((record + 0x9) as *const u8) };
    // SAFETY: `+0xa` is the header's image type (2 = uncompressed true color).
    let image_type = unsafe { *((record + 0xa) as *const u8) };
    // SAFETY: `+0x4` is the pixel-data pointer.
    let data = unsafe { *((record + 0x4) as *const usize) };
    if data == 0 || image_type != 2 || extra_header != 0 || color_map != 0 {
        return false;
    }
    // SAFETY: `+0x14` is the header's width (header base `+0xc`).
    let width = unsafe { *((record + 0x14) as *const u16) };
    // SAFETY: `+0x16` is the header's height.
    let height = unsafe { *((record + 0x16) as *const u16) };
    if width == 0 || height == 0 {
        return false;
    }
    // SAFETY: the writer object owns a NUL-terminated path string.
    let stock_path = unsafe { core::ffi::CStr::from_ptr(tga_filename.cast()) };
    let stock_path = PathBuf::from(String::from_utf8_lossy(stock_path.to_bytes()).into_owned());
    let (Some(directory), Some(stem)) = (stock_path.parent(), stock_path.file_stem()) else {
        return false;
    };
    let count = COUNTER.load(Ordering::Relaxed) % 999 + 1;
    COUNTER.store(count, Ordering::Relaxed);
    let png = super::settings::screenshot_png();
    let mut name = stem.to_os_string();
    name.push(format!("_{count:03}.{}", if png { "png" } else { "jpg" }));
    let target = directory.join(name);
    let total = usize::from(width) * usize::from(height) * BYTES_PER_PIXEL;
    // SAFETY: the frame buffer holds `width * height` packed 24-bit pixels,
    // the exact region the stock writer would serialize.
    let pixels = unsafe { core::slice::from_raw_parts(data as *const u8, total) }.to_vec();
    let width = u32::from(width);
    let height = u32::from(height);
    std::thread::spawn(move || {
        let mut pixels = pixels;
        for pixel in pixels.chunks_exact_mut(BYTES_PER_PIXEL) {
            pixel.swap(0, 2);
        }
        if png {
            write_png(&target, width, height, &pixels);
        } else {
            write_jpeg(&target, width, height, &pixels);
        }
    });
    true
}

fn write_jpeg(target: &std::path::Path, width: u32, height: u32, rgb: &[u8]) {
    let Ok(encoder) = jpeg_encoder::Encoder::new_file(target, JPEG_QUALITY) else {
        return;
    };
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return;
    };
    let _ = encoder.encode(rgb, width, height, jpeg_encoder::ColorType::Rgb);
}

/// A minimal PNG writer: one IDAT chunk of filter-0 rows.
///
/// The zlib stream comes from the vendored deflate.
fn write_png(target: &std::path::Path, width: u32, height: u32, rgb: &[u8]) {
    let stride = width as usize * BYTES_PER_PIXEL;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgb.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut out = Vec::with_capacity(compressed.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut header = [0u8; 13];
    header[..4].copy_from_slice(&width.to_be_bytes());
    header[4..8].copy_from_slice(&height.to_be_bytes());
    // Bit depth 8, color type 2 (truecolor), default compression/filter/interlace.
    header[8] = 8;
    header[9] = 2;
    push_chunk(&mut out, b"IHDR", &header);
    push_chunk(&mut out, b"IDAT", &compressed);
    push_chunk(&mut out, b"IEND", &[]);
    let _ = std::fs::write(target, out);
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("a screenshot chunk is far below 4 GiB")
            .to_be_bytes(),
    );
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(payload);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// The PNG chunk checksum (reflected CRC-32, byte-table form).
struct Crc32(u32);

impl Crc32 {
    /// The reflected CRC-32 polynomial the PNG format fixes.
    const POLYNOMIAL: u32 = 0xedb8_8320;

    /// One fold per byte value, built from [`Self::POLYNOMIAL`] at compile time.
    ///
    /// A byte-wide table answers a payload byte in one load and one shift/xor
    /// pair. The 16-entry nibble table it replaces fits in a sixteenth of the
    /// read-only data and costs two loads and twice the arithmetic per byte,
    /// over a payload that is the whole compressed image. Both forms fold the
    /// same polynomial, so the checksum is unchanged.
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut value = 0u32;
        while value < 256 {
            let mut fold = value;
            let mut bit = 0;
            while bit < 8 {
                fold = if fold & 1 == 0 {
                    fold >> 1
                } else {
                    Self::POLYNOMIAL ^ (fold >> 1)
                };
                bit += 1;
            }
            table[value as usize] = fold;
            value += 1;
        }
        table
    };

    const fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = Self::TABLE[((self.0 ^ u32::from(byte)) & 0xff) as usize] ^ (self.0 >> 8);
        }
    }

    const fn finish(&self) -> u32 {
        self.0 ^ 0xffff_ffff
    }
}
