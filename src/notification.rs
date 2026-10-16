/// A notification, as a web service worker's Rust handler builds it for a push, and as a Linux or
/// Windows app shows it with [`show`](Notification::show).
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
    /// A notification with this title, which must not be empty.
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

    /// The URL opened when the user clicks the notification.
    #[must_use]
    pub fn navigate(mut self, url: impl Into<String>) -> Self {
        self.navigate = Some(url.into());
        self
    }

    /// The tag under which a later notification replaces this one.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// The URL of the icon.
    #[must_use]
    pub fn icon(mut self, url: impl Into<String>) -> Self {
        self.icon = Some(url.into());
        self
    }

    /// What a tap on the notification hands back, as the payload of an [`Event::Message`](crate::Event::Message)
    /// on Linux and Windows. A tap on the web opens or focuses a page and delivers no event.
    ///
    /// Windows refuses a toast past about 5 KB, which with a short title leaves room for about
    /// 3.5 KB of `data`, and [`show`](Notification::show) then fails.
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
