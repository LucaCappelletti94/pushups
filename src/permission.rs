/// The user's answer to the notification permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Permission {
    /// Pushes may show notifications.
    Granted,
    /// The user refused, or the system refuses without asking.
    Denied,
    /// The prompt was closed without an answer, so the app may show it again. Only the web
    /// reports it.
    Dismissed,
}
