use crate::{Error, Token};

/// What the platform hands to the handler set with [`set_handler`](crate::set_handler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A new or refreshed token, to be sent to the app's server.
    Token(Token),
    /// The platform could not produce a token.
    RegistrationFailed(Error),
    /// A push arrived.
    Message(Message),
}

/// A received push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The push payload as the platform delivered it.
    pub payload: Vec<u8>,
    /// Whether the operating system started the app to deliver this push.
    pub started_app: bool,
}
