//! The window's content, the app's title and its latest lines drawn with an 8×8 bitmap font
//! through `softbuffer`, since winit draws nothing itself.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;

use pushups_example_support::text::{self, Frame, Insets};
use softbuffer::{Context, Surface};
use winit::window::Window;

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
        let (Ok(columns), Ok(rows)) = (usize::try_from(size.width), usize::try_from(size.height))
        else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        let frame = Frame {
            width: columns,
            height: rows,
            scale: text::scale(window.scale_factor()),
            insets: insets(window.as_ref()),
        };
        text::draw(
            &mut buffer,
            &frame,
            TITLE_TEXT,
            self.lines.iter().map(String::as_str),
        );
        let _ = buffer.present();
    }
}

/// The edges system bars cover, from the safe area winit reports, or on Android, where winit
/// reports none, from the Activity's content rectangle.
fn insets(window: &dyn Window) -> Insets {
    let edge = |pixels: i64| usize::try_from(pixels).unwrap_or(0);
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::WindowExtAndroid as _;

        let size = window.surface_size();
        let rect = window.content_rect();
        Insets {
            top: edge(rect.top.into()),
            left: edge(rect.left.into()),
            bottom: edge(i64::from(size.height) - i64::from(rect.bottom)),
            right: edge(i64::from(size.width) - i64::from(rect.right)),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let area = window.safe_area();
        Insets {
            top: edge(area.top.into()),
            left: edge(area.left.into()),
            bottom: edge(area.bottom.into()),
            right: edge(area.right.into()),
        }
    }
}
