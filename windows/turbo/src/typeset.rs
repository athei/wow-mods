//! Single-line text rasterization for the combat-text overlay.
//!
//! Wraps `ab_glyph` over a TTF loaded from the client's own `Fonts/`
//! directory: measure a line, ask whether a face covers every character, and
//! rasterize a line once into an 8-bit coverage bitmap the adapter uploads
//! into a texture. Combat text is single-line and unshaped, so layout is a
//! left-to-right pen with kerning — no wrapping, no bidi. The pixel size is
//! treated as the em height, which runs a couple of pixels larger than the
//! cell-height interpretation the reference's GDI fonts used at the same
//! number; the size is user-tunable, so the constant offset is accepted
//! rather than compensated.

use ab_glyph::{Font, FontVec, ScaleFont};

/// A rasterized line: row-major 8-bit coverage, `width * height` bytes.
pub struct Raster {
    /// Bitmap width in pixels, at least one.
    pub width: u32,
    /// Bitmap height in pixels, at least one.
    pub height: u32,
    /// Row-major coverage, `0` transparent to `255` fully inked.
    pub coverage: Vec<u8>,
}

/// A loaded typeface.
pub struct Face {
    font: FontVec,
}

impl Face {
    /// Parse a TTF/OTF; `None` when the bytes are not a parseable font.
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        FontVec::try_from_vec(data).ok().map(|font| Self { font })
    }

    /// Whether the face has a real glyph for every character of `text`.
    ///
    /// Index zero is the `.notdef` box; a line failing this check is handed
    /// back to the stock renderer rather than drawn with tofu.
    pub fn covers(&self, text: &str) -> bool {
        text.chars()
            .all(|c| c.is_control() || self.font.glyph_id(c).0 != 0)
    }

    /// The line's pixel width and cell height at `px`, without rasterizing.
    ///
    /// `kern` and `h_advance` each re-derive the scale factor `px / (ascender -
    /// descender)` from a fresh pair of metric-table reads, so it is taken once
    /// for the line and the `*_unscaled` metrics are scaled here. `sf *
    /// unscaled` is the library's own operand order, so every product keeps its
    /// bits. The cell height stays `height()`, which is the pixel size by
    /// definition and must not be re-derived through the factor.
    pub fn measure(&self, text: &str, px: f32) -> (i32, i32) {
        let scaled = self.font.as_scaled(px);
        let sf = scaled.h_scale_factor();
        let mut pen = 0.0f32;
        let mut previous = None;
        for c in text.chars() {
            if c.is_control() {
                continue;
            }
            let id = self.font.glyph_id(c);
            if let Some(prev) = previous {
                pen += sf * self.font.kern_unscaled(prev, id);
            }
            pen += sf * self.font.h_advance_unscaled(id);
            previous = Some(id);
        }
        (ceil_px(pen), ceil_px(scaled.height()))
    }

    /// Rasterize the line at `px` into a coverage bitmap.
    ///
    /// Glyphs overhanging the advance box (italic tails, tight kerns) are
    /// clipped by one padding pixel each side rather than measured exactly;
    /// the shadow offset under every drawn line hides more than that.
    ///
    /// The scale factor is taken once for the line, as in `measure`.
    pub fn rasterize(&self, text: &str, px: f32) -> Raster {
        let scaled = self.font.as_scaled(px);
        let (text_w, text_h) = self.measure(text, px);
        let pad = 1i32;
        let width = u32::try_from(text_w + pad * 2).map_or(1, |w| w.max(1));
        let height = u32::try_from(text_h + pad * 2).map_or(1, |h| h.max(1));
        let mut coverage = vec![0u8; (width as usize) * (height as usize)];

        let ascent = scaled.ascent();
        let sf = scaled.h_scale_factor();
        let mut pen = to_px_f32(pad);
        let mut previous = None;
        for c in text.chars() {
            if c.is_control() {
                continue;
            }
            let id = self.font.glyph_id(c);
            if let Some(prev) = previous {
                pen += sf * self.font.kern_unscaled(prev, id);
            }
            let glyph =
                id.with_scale_and_position(px, ab_glyph::point(pen, ascent + to_px_f32(pad)));
            pen += sf * self.font.h_advance_unscaled(id);
            previous = Some(id);

            let Some(outlined) = self.font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            let origin_x = floor_px(bounds.min.x);
            let origin_y = floor_px(bounds.min.y);
            outlined.draw(|gx, gy, c| {
                let x = origin_x + i32::try_from(gx).unwrap_or(i32::MAX);
                let y = origin_y + i32::try_from(gy).unwrap_or(i32::MAX);
                if x < 0 || y < 0 {
                    return;
                }
                let (x, y) = (x.cast_unsigned(), y.cast_unsigned());
                if x >= width || y >= height {
                    return;
                }
                let value = coverage_byte(c);
                let cell = &mut coverage[(y as usize) * (width as usize) + (x as usize)];
                *cell = cell.saturating_add(value);
            });
        }

        Raster {
            width,
            height,
            coverage,
        }
    }
}

/// Coverage fraction to a byte, saturating, rounding half up.
fn coverage_byte(c: f32) -> u8 {
    let scaled = c * 255.0 + 0.5;
    if scaled >= 255.0 {
        255
    } else if scaled <= 0.0 {
        0
    } else {
        // Range-checked right above; the truncation is the rounding.
        // range-checked above
        scaled as u8
    }
}

/// `f32` pixel measure to `i32`, ceiling, saturating.
fn ceil_px(v: f32) -> i32 {
    let up = v.ceil();
    if up >= 2_147_483_647.0 {
        i32::MAX
    } else if up <= 0.0 {
        0
    } else {
        // Range-checked right above.
        // range-checked above
        up as i32
    }
}

/// `f32` pixel coordinate to `i32`, flooring, saturating.
fn floor_px(v: f32) -> i32 {
    let down = v.floor();
    if down >= 2_147_483_647.0 {
        i32::MAX
    } else if down <= -2_147_483_648.0 {
        i32::MIN
    } else {
        // Range-checked right above.
        // range-checked above
        down as i32
    }
}

/// Small positive `i32` to `f32`, exact for any value this module produces.
fn to_px_f32(v: i32) -> f32 {
    // Padding and pen offsets are tiny; the conversion is exact well past
    // every realistic text width.
    // values far below 2^24
    v as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system font for smoke tests, wherever one of the candidates exists.
    ///
    /// The suite runs on the development host; when no candidate exists the
    /// smoke tests skip rather than fail, and the pure helpers below still
    /// run everywhere.
    fn system_face() -> Option<Face> {
        let candidates = [
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Tahoma.ttf",
            "/Library/Fonts/Arial.ttf",
        ];
        candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok())
            .and_then(Face::from_bytes)
    }

    #[test]
    fn garbage_bytes_are_not_a_face() {
        assert!(Face::from_bytes(vec![0u8; 64]).is_none());
    }

    #[test]
    fn coverage_byte_rounds_and_saturates() {
        assert_eq!(coverage_byte(0.0), 0);
        assert_eq!(coverage_byte(1.0), 255);
        assert_eq!(coverage_byte(0.5), 128);
        assert_eq!(coverage_byte(-1.0), 0);
        assert_eq!(coverage_byte(2.0), 255);
    }

    #[test]
    fn measure_and_rasterize_agree() {
        let Some(face) = system_face() else {
            return;
        };
        let (w, h) = face.measure("1234 crit", 36.0);
        assert!(w > 0 && h > 0);
        let raster = face.rasterize("1234 crit", 36.0);
        // The bitmap is the measured box plus one padding pixel per side.
        assert_eq!(raster.width, u32::try_from(w + 2).expect("small"));
        assert_eq!(raster.height, u32::try_from(h + 2).expect("small"));
        assert_eq!(
            raster.coverage.len(),
            (raster.width * raster.height) as usize
        );
        // Something was actually inked.
        assert!(raster.coverage.iter().any(|&c| c > 0));
    }

    /// The hoisted factor reproduces the per-call accessors bit for bit.
    ///
    /// `kern`/`h_advance` are `h_scale_factor() * unscaled`, so taking the
    /// factor once for the line and writing the product in the same operand
    /// order has to give the identical `f32` for every glyph and pair.
    #[test]
    fn hoisted_scale_factor_matches_the_accessors() {
        let Some(face) = system_face() else {
            return;
        };
        let scaled = face.font.as_scaled(36.0f32);
        let sf = scaled.h_scale_factor();
        let ids: Vec<_> = "1234 critAVWy."
            .chars()
            .map(|c| face.font.glyph_id(c))
            .collect();
        for &id in &ids {
            assert_eq!(
                (sf * face.font.h_advance_unscaled(id)).to_bits(),
                scaled.h_advance(id).to_bits()
            );
            for &prev in &ids {
                assert_eq!(
                    (sf * face.font.kern_unscaled(prev, id)).to_bits(),
                    scaled.kern(prev, id).to_bits()
                );
            }
        }
    }

    #[test]
    fn wider_text_measures_wider() {
        let Some(face) = system_face() else {
            return;
        };
        let (short, _) = face.measure("12", 36.0);
        let (long, _) = face.measure("123456", 36.0);
        assert!(long > short);
    }

    #[test]
    fn coverage_check_flags_missing_glyphs() {
        let Some(face) = system_face() else {
            return;
        };
        assert!(face.covers("Dodge 123"));
        // A Latin system face has no glyph for a CJK ideograph.
        assert!(!face.covers("\u{7ecf}\u{9a8c}"));
    }
}
