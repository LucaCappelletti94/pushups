//! Where an Android event goes, by the state of the app's UI, as the plan's Android delivery
//! table lays out.

/// The UI state of the process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Session {
    /// No Activity is alive.
    #[default]
    Headless,
    /// No Activity is alive and the handler set during the UI session that just ended is still
    /// set. It binds the next session only while the thread that set it still runs.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        expect(dead_code, reason = "only Android reports the end of a UI session")
    )]
    HeadlessKept,
    /// No Activity is alive and a handler was set since the last UI session ended, as an
    /// android-activity app's `android_main` does before its Activity reports itself created.
    HeadlessHandled,
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
    /// A UI session began (`true`) or ended (`false`), which only Android reports.
    /// `kept_handler_alive` tells whether the thread that set the current handler still runs.
    /// Returns the new state and whether the queue drains into the handler already set.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn on_session(self, active: bool, kept_handler_alive: bool) -> (Self, bool) {
        match (self, active) {
            (Self::HeadlessKept, true) if kept_handler_alive => (Self::Bound, true),
            (Self::Headless | Self::HeadlessKept, true) => (Self::Unbound, false),
            (Self::HeadlessHandled, true) => (Self::Bound, true),
            (Self::Bound, false) => (Self::HeadlessKept, false),
            (Self::Unbound, false) => (Self::Headless, false),
            (session, _) => (session, false),
        }
    }

    /// A handler was set. Returns the new state and whether the queue drains into it.
    pub(crate) fn on_handler_set(self) -> (Self, bool) {
        match self {
            Self::Headless | Self::HeadlessKept => (Self::HeadlessHandled, false),
            Self::Unbound => (Self::Bound, true),
            session => (session, false),
        }
    }

    pub(crate) fn route(self) -> Route {
        match self {
            Self::Bound => Route::Emit,
            Self::Headless | Self::HeadlessKept | Self::HeadlessHandled | Self::Unbound => {
                Route::Persist
            }
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
    fn a_handler_set_without_a_live_activity_binds_the_next_ui_session_and_drains_it() {
        let (handled, drain) = Session::Headless.on_handler_set();
        assert_eq!((handled, drain), (Session::HeadlessHandled, false));
        assert_eq!(handled.route(), Route::Persist);
        assert_eq!(handled.on_handler_set(), (Session::HeadlessHandled, false));
        let (bound, drain) = handled.on_session(true, false);
        assert_eq!((bound, drain), (Session::Bound, true));
        assert_eq!(bound.route(), Route::Emit);
    }

    #[test]
    fn a_new_ui_session_persists_until_its_handler_is_set_then_drains_once() {
        let (unbound, drain) = Session::Headless.on_session(true, true);
        assert_eq!((unbound, drain), (Session::Unbound, false));
        assert_eq!(unbound.route(), Route::Persist);
        let (bound, drain) = unbound.on_handler_set();
        assert_eq!((bound, drain), (Session::Bound, true));
        assert_eq!(bound.route(), Route::Emit);
        assert_eq!(bound.on_handler_set(), (Session::Bound, false));
    }

    #[test]
    fn closing_the_ui_keeps_its_handler_for_the_next_session_while_the_setting_thread_runs() {
        let (kept, drain) = Session::Bound.on_session(false, true);
        assert_eq!((kept, drain), (Session::HeadlessKept, false));
        assert_eq!(kept.route(), Route::Persist);
        let (bound, drain) = kept.on_session(true, true);
        assert_eq!((bound, drain), (Session::Bound, true));
        assert_eq!(bound.route(), Route::Emit);
    }

    #[test]
    fn a_kept_handler_whose_thread_exited_binds_no_later_session() {
        let (unbound, drain) = Session::HeadlessKept.on_session(true, false);
        assert_eq!((unbound, drain), (Session::Unbound, false));
        assert_eq!(unbound.route(), Route::Persist);
        assert_eq!(unbound.on_handler_set(), (Session::Bound, true));
    }

    #[test]
    fn a_handler_set_while_one_is_kept_binds_the_next_session_whatever_its_thread() {
        let (handled, drain) = Session::HeadlessKept.on_handler_set();
        assert_eq!((handled, drain), (Session::HeadlessHandled, false));
        assert_eq!(handled.on_session(true, false), (Session::Bound, true));
    }

    #[test]
    fn closing_a_ui_whose_session_set_no_handler_returns_to_headless() {
        assert_eq!(
            Session::Unbound.on_session(false, true),
            (Session::Headless, false)
        );
    }

    #[test]
    fn a_repeated_session_report_keeps_the_state() {
        for session in [Session::Bound, Session::Unbound] {
            assert_eq!(session.on_session(true, false), (session, false));
        }
        for session in [
            Session::Headless,
            Session::HeadlessKept,
            Session::HeadlessHandled,
        ] {
            assert_eq!(session.on_session(false, false), (session, false));
        }
    }
}
