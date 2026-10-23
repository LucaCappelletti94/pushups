/// Notification content for a service worker, an iOS service extension or native presentation.
///
/// The fields follow the `notification` member of a Declarative Web Push message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notification {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) navigate: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) data: Option<String>,
}

impl Notification {
    /// A notification with this title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// The text under the title.
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// The navigation URL, stored in the rendered user info on iOS.
    #[must_use]
    pub fn navigate(mut self, url: impl Into<String>) -> Self {
        self.navigate = Some(url.into());
        self
    }

    /// The replacement tag for presentation, or the iOS extension's thread identifier.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// The icon URL, unsupported by mobile rendering and presentation.
    #[must_use]
    pub fn icon(mut self, url: impl Into<String>) -> Self {
        self.icon = Some(url.into());
        self
    }

    /// The UTF-8 payload a native notification tap delivers as [`Event::Message`](crate::Event::Message).
    ///
    /// Web notification taps open or focus a page without delivering an event.
    ///
    /// Windows refuses a toast past about 5 KB, which with a short title leaves room for about
    /// 3.5 KB of `data`, and `Notification::show` then fails.
    #[must_use]
    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[cfg_attr(docsrs, doc(cfg(any(target_os = "linux", target_os = "windows"))))]
impl Notification {
    /// Shows the notification in the desktop's notification centre, where the system shows
    /// nothing for a push by itself.
    ///
    /// A tap on a notification with [`data`](Notification::data) reaches the app as an
    /// [`Event::Message`](crate::Event::Message) carrying it, and starts the app when it has exited.
    ///
    /// # Errors
    ///
    /// [`Error::NotConfigured`](crate::Error::NotConfigured) before [`install`](crate::install),
    /// and on Windows without [`WnsConfig::app_notifications`](crate::WnsConfig::app_notifications).
    /// [`Error::Platform`](crate::Error::Platform) when the notification service refuses the
    /// notification or does not answer within five seconds.
    pub fn show(&self) -> Result<(), crate::Error> {
        #[cfg(target_os = "linux")]
        let shown = crate::linux::show(self);
        #[cfg(target_os = "windows")]
        let shown = crate::windows::show(self);
        shown
    }
}
