//! Where an Android event goes, by the state of the app's UI, as the plan's Android delivery
//! table lays out.

/// The UI state of the process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Session {
    /// No Activity is alive.
    #[default]
    Headless,
    /// An Activity is alive and no handler was set since its session began.
    Unbound,
    /// An Activity is alive and a handler was set during its session.
    Bound,
}

/// What happens to an event arriving from the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// Written to the queue for a later handler.
    Persist,
    /// Handed to the dispatcher now.
    Emit,
}

impl Session {
    /// A UI session began (`true`) or ended (`false`).
    pub(crate) fn on_session(self, active: bool) -> Self {
        match (self, active) {
            (Self::Headless, true) => Self::Unbound,
            (_, false) => Self::Headless,
            (session, true) => session,
        }
    }

    /// A handler was set. Returns the new state and whether the queue drains into it.
    pub(crate) fn on_handler_set(self) -> (Self, bool) {
        match self {
            Self::Unbound => (Self::Bound, true),
            session => (session, false),
        }
    }

    pub(crate) fn route(self) -> Route {
        match self {
            Self::Bound => Route::Emit,
            Self::Headless | Self::Unbound => Route::Persist,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_starts_headless_and_persists() {
        assert_eq!(Session::default(), Session::Headless);
        assert_eq!(Session::Headless.route(), Route::Persist);
    }

    #[test]
    fn a_handler_set_without_a_live_activity_drains_nothing() {
        assert_eq!(
            Session::Headless.on_handler_set(),
            (Session::Headless, false)
        );
    }

    #[test]
    fn a_new_ui_session_persists_until_its_handler_is_set_then_drains_once() {
        let unbound = Session::Headless.on_session(true);
        assert_eq!(unbound, Session::Unbound);
        assert_eq!(unbound.route(), Route::Persist);
        let (bound, drain) = unbound.on_handler_set();
        assert_eq!((bound, drain), (Session::Bound, true));
        assert_eq!(bound.route(), Route::Emit);
        assert_eq!(bound.on_handler_set(), (Session::Bound, false));
    }

    #[test]
    fn closing_the_ui_returns_to_persisting_and_the_old_handler_does_not_bind_the_next_session() {
        for session in [Session::Unbound, Session::Bound] {
            assert_eq!(session.on_session(false), Session::Headless);
        }
        let next = Session::Bound.on_session(false).on_session(true);
        assert_eq!(next, Session::Unbound);
        assert_eq!(next.route(), Route::Persist);
    }

    #[test]
    fn a_repeated_session_start_keeps_the_binding() {
        assert_eq!(Session::Bound.on_session(true), Session::Bound);
        assert_eq!(Session::Unbound.on_session(true), Session::Unbound);
        assert_eq!(Session::Headless.on_session(false), Session::Headless);
    }
}
