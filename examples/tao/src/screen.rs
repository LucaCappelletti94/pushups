//! The window's content, the app's title and its latest lines drawn with an 8×8 bitmap font
//! through `softbuffer`, since tao draws nothing itself.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;

use pushups_example_support::text::{self, Frame, Insets};
use softbuffer::{Context, Surface};
use tao::window::Window;

const TITLE_TEXT: &str = "pushups tao example";
/// The lines kept, more than any window shows once they wrap.
const KEPT: usize = 32;

/// The lines the window shows, and the surface they are drawn on.
pub(crate) struct Screen {
    /// The permission state, the hint while asking is offered, drawn above the lines.
    state: Option<String>,
    lines: VecDeque<String>,
    surface: Surface<Rc<Window>, Rc<Window>>,
}

impl Screen {
    /// Draws into `window`.
    pub(crate) fn new(window: Rc<Window>) -> Result<Self, softbuffer::SoftBufferError> {
        let context = Context::new(Rc::clone(&window))?;
        Ok(Self {
            state: None,
            lines: VecDeque::new(),
            surface: Surface::new(&context, window)?,
        })
    }

    /// Adds a line under the others and asks for a redraw.
    pub(crate) fn push(&mut self, line: String) {
        if self.lines.len() == KEPT {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.surface.window().request_redraw();
    }

    /// Sets the permission state drawn above the lines and asks for a redraw.
    pub(crate) fn set_state(&mut self, state: String) {
        self.state = Some(state);
        self.surface.window().request_redraw();
    }

    /// Draws the title and as many of the newest lines as fit.
    pub(crate) fn draw(&mut self) {
        let window = Rc::clone(self.surface.window());
        let (size, insets) = area(&window);
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        let (Ok(columns), Ok(rows)) = (usize::try_from(size.width), usize::try_from(size.height))
        else {
            return;
        };
        if self.surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return;
        };
        let frame = Frame {
            width: columns,
            height: rows,
            scale: text::scale(window.scale_factor()),
            insets,
        };
        let lines = self
            .state
            .iter()
            .map(String::as_str)
            .chain(self.lines.iter().map(String::as_str));
        text::draw(&mut buffer, &frame, TITLE_TEXT, lines);
        let _ = buffer.present();
    }
}

/// The size of the surface and the edges system bars cover. On iOS the view fills the screen and
/// tao's inner area is the safe area within it. Elsewhere the surface is the inner area.
fn area(window: &Window) -> (tao::dpi::PhysicalSize<u32>, Insets) {
    #[cfg(target_os = "ios")]
    {
        let outer = window.outer_size();
        let inner = window.inner_size();
        let (Ok(outer_at), Ok(inner_at)) = (window.outer_position(), window.inner_position())
        else {
            return (outer, Insets::default());
        };
        let edge = |pixels: i64| usize::try_from(pixels).unwrap_or(0);
        let top = i64::from(inner_at.y) - i64::from(outer_at.y);
        let left = i64::from(inner_at.x) - i64::from(outer_at.x);
        let insets = Insets {
            top: edge(top),
            left: edge(left),
            bottom: edge(i64::from(outer.height) - i64::from(inner.height) - top),
            right: edge(i64::from(outer.width) - i64::from(inner.width) - left),
        };
        (outer, insets)
    }
    #[cfg(not(target_os = "ios"))]
    {
        (window.inner_size(), Insets::default())
    }
}
