/// The notification a service worker shows for a push, as the Rust handler builds it.
///
/// The fields follow the `notification` member of a Declarative Web Push message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notification {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) navigate: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) icon: Option<String>,
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
}
