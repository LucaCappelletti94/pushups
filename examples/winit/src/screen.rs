//! The window's content, the app's title and its latest lines drawn with an 8×8 bitmap font
//! through `softbuffer`, since winit draws nothing itself.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;

use font8x8::UnicodeFonts as _;
use softbuffer::{Context, Surface};
use winit::window::Window;

const BACKGROUND: u32 = 0x000e_1824;
const TITLE: u32 = 0x00f6_7017;
const TEXT: u32 = 0x00f5_f7fa;
const TITLE_TEXT: &str = "pushups winit example";
/// The lines kept, more than any window shows once they wrap.
const KEPT: usize = 32;

/// The lines the window shows, and the surface they are drawn on while the window has one.
pub(crate) struct Screen {
    lines: VecDeque<String>,
    surface: Option<Surface<Rc<dyn Window>, Rc<dyn Window>>>,
}

impl Screen {
    pub(crate) fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            surface: None,
        }
    }

    /// Draws into `window` from now on.
    pub(crate) fn attach(
        &mut self,
        window: Rc<dyn Window>,
    ) -> Result<(), softbuffer::SoftBufferError> {
        let context = Context::new(Rc::clone(&window))?;
        self.surface = Some(Surface::new(&context, window)?);
        Ok(())
    }

    /// Drops the surface, which Android requires once its native window is gone.
    pub(crate) fn detach(&mut self) {
        self.surface = None;
    }

    /// Adds a line under the others and asks for a redraw.
    pub(crate) fn push(&mut self, line: String) {
        if self.lines.len() == KEPT {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        if let Some(surface) = &self.surface {
            surface.window().request_redraw();
        }
    }

    /// Draws the title and as many of the newest lines as fit.
    pub(crate) fn draw(&mut self) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        let window = Rc::clone(surface.window());
        let size = window.surface_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        buffer.fill(BACKGROUND);
        let (width, height) = (size.width as usize, size.height as usize);
        let scale = ((window.scale_factor() * 2.0).round() as usize).max(1);
        let glyph = 8 * scale;
        let row = glyph + 4 * scale;
        let margin = 2 * glyph;
        let columns = width.saturating_sub(2 * margin) / glyph;
        if columns == 0 {
            let _ = buffer.present();
            return;
        }
        let mut canvas = Canvas {
            pixels: &mut buffer,
            width,
            height,
            scale,
        };
        canvas.text(margin, margin, TITLE_TEXT, TITLE);
        let top = margin + 2 * row;
        let fitting = height.saturating_sub(top + margin) / row;
        let rows: Vec<&str> = self
            .lines
            .iter()
            .flat_map(|line| wrap(line, columns))
            .collect();
        for (index, text) in rows[rows.len().saturating_sub(fitting)..]
            .iter()
            .enumerate()
        {
            canvas.text(margin, top + index * row, text, TEXT);
        }
        let _ = buffer.present();
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
