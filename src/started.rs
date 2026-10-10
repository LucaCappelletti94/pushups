//! Whether an Apple event started the app, read from the process's activation state as the plan's Decided table says.

/// Which kind of event the system handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// The system delivered a push.
    Push,
    /// The user tapped a notification.
    Tap,
}

/// The activation state of this process, as far as `started_app` needs it.
#[derive(Debug, Default)]
pub(crate) struct Started {
    launched_in_background: bool,
    ever_active: bool,
    first_event_seen: bool,
}

impl Started {
    /// The process finished launching, in the background when the system started it for a push.
    pub(crate) fn launched(&mut self, in_background: bool) {
        self.launched_in_background = in_background;
    }

    /// The app became active, so no later event started it.
    pub(crate) fn became_active(&mut self) {
        self.ever_active = true;
    }

    /// Whether the event arriving now started the app. Only a process's first event can have.
    pub(crate) fn started_app(&mut self, arrival: Arrival) -> bool {
        let first = !self.first_event_seen && !self.ever_active;
        self.first_event_seen = true;
        first
            && match arrival {
                Arrival::Push => self.launched_in_background,
                Arrival::Tap => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_push_of_a_background_launch_started_the_app() {
        let mut started = Started::default();
        started.launched(true);
        assert!(started.started_app(Arrival::Push));
        assert!(!started.started_app(Arrival::Push), "only the first event");
    }

    #[test]
    fn a_tap_that_comes_before_the_app_was_active_started_it() {
        let mut started = Started::default();
        started.launched(false);
        assert!(started.started_app(Arrival::Tap));
        assert!(!started.started_app(Arrival::Tap));
    }

    #[test]
    fn nothing_starts_an_app_that_was_already_active() {
        let mut started = Started::default();
        started.launched(true);
        started.became_active();
        assert!(!started.started_app(Arrival::Push));
        let mut started = Started::default();
        started.launched(false);
        started.became_active();
        assert!(!started.started_app(Arrival::Tap));
    }

    #[test]
    fn a_push_during_a_launch_in_front_did_not_start_the_app() {
        let mut started = Started::default();
        started.launched(false);
        assert!(!started.started_app(Arrival::Push));
        assert!(
            !started.started_app(Arrival::Tap),
            "the push was the first event"
        );
    }
}
