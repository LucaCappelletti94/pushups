//! The window content the native examples draw with an 8×8 bitmap font, since winit and tao
//! draw nothing themselves, the title and as many of the newest lines as fit.

use font8x8::UnicodeFonts as _;

const BACKGROUND: u32 = 0x000e_1824;
const TITLE: u32 = 0x00f6_7017;
const TEXT: u32 = 0x00f5_f7fa;

/// A pixel buffer of `width` by `height`, with the edges system bars cover.
pub struct Frame {
    /// Pixels per row.
    pub width: usize,
    /// Rows.
    pub height: usize,
    /// Pixels per font dot.
    pub scale: usize,
    /// The edges kept clear.
    pub insets: Insets,
}

/// The pixels each edge of a [`Frame`] keeps clear.
#[derive(Default)]
pub struct Insets {
    /// Rows at the top.
    pub top: usize,
    /// Columns at the left.
    pub left: usize,
    /// Rows at the bottom.
    pub bottom: usize,
    /// Columns at the right.
    pub right: usize,
}

/// Pixels per font dot for a display scale factor, one and a half dots per logical pixel, at
/// least one.
#[must_use]
pub fn scale(factor: f64) -> usize {
    let dots = (factor * 1.5).round();
    debug_assert!(dots.is_finite(), "a window's scale factor is finite");
    // A scale factor is a small positive number, so the clamped rounded value converts exactly.
    dots.clamp(1.0, 64.0) as usize
}

/// Fills `pixels`, `frame.width` by `frame.height` in `0x00RRGGBB`, with the title and the newest
/// `lines` that fit, each wrapped to the width.
pub fn draw<'line>(
    pixels: &mut [u32],
    frame: &Frame,
    title: &str,
    lines: impl IntoIterator<Item = &'line str>,
) {
    pixels.fill(BACKGROUND);
    let glyph = 8 * frame.scale;
    let row = glyph + 4 * frame.scale;
    let left = frame.insets.left + 2 * glyph;
    let top = frame.insets.top + 2 * glyph;
    let bottom = frame.height.saturating_sub(frame.insets.bottom + 2 * glyph);
    let columns = frame
        .width
        .saturating_sub(left + frame.insets.right + 2 * glyph)
        / glyph;
    if columns == 0 {
        return;
    }
    let mut canvas = Canvas {
        pixels,
        width: frame.width,
        height: frame.height,
        scale: frame.scale,
    };
    let mut y = top;
    for text in wrap(title, columns) {
        canvas.text(left, y, text, TITLE);
        y += row;
    }
    y += row;
    let fitting = bottom.saturating_sub(y) / row;
    let rows: Vec<&str> = lines
        .into_iter()
        .flat_map(|line| wrap(line, columns))
        .collect();
    for text in &rows[rows.len().saturating_sub(fitting)..] {
        canvas.text(left, y, text, TEXT);
        y += row;
    }
}

/// The line cut into pieces of at most `columns` characters, at a space where one falls inside.
fn wrap(line: &str, columns: usize) -> impl Iterator<Item = &str> {
    let mut rest = line;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let Some((limit, next)) = rest.char_indices().nth(columns) else {
            return Some(std::mem::take(&mut rest));
        };
        let space = if next == ' ' {
            Some(limit)
        } else {
            rest[..limit].rfind(' ').filter(|&space| space > 0)
        };
        let (piece, tail) = match space {
            Some(space) => (&rest[..space], &rest[space + 1..]),
            None => rest.split_at(limit),
        };
        rest = tail;
        Some(piece)
    })
}

struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    scale: usize,
}

impl Canvas<'_> {
    /// Draws `text` with its top left corner at `x`, `y`, clipped to the buffer.
    fn text(&mut self, x: usize, y: usize, text: &str, color: u32) {
        let glyph = 8 * self.scale;
        for (index, character) in text.chars().enumerate() {
            let rows = font8x8::BASIC_FONTS
                .get(character)
                .or_else(|| font8x8::BASIC_FONTS.get('?'))
                .unwrap_or_default();
            let left = x + index * glyph;
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..8 {
                    if bits & (1 << column) != 0 {
                        self.block(left + column * self.scale, y + row * self.scale, color);
                    }
                }
            }
        }
    }

    fn block(&mut self, x: usize, y: usize, color: u32) {
        for row in y..(y + self.scale).min(self.height) {
            let start = row * self.width + x.min(self.width);
            let end = row * self.width + (x + self.scale).min(self.width);
            self.pixels[start..end].fill(color);
        }
    }
}
