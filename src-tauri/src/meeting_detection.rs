use std::collections::BTreeSet;

const CLEAR_AFTER_INACTIVE_POLLS: u8 = 2;
const HEARTBEAT_EVERY_ACTIVE_POLLS: u8 = 5;

pub fn setup(_app: &mut tauri::App) {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeetingDetectionEvent {
    Detected,
    Cleared,
}

#[derive(Debug, Default)]
pub(crate) struct MeetingDetectionState {
    active: bool,
    inactive_polls: u8,
    active_polls_since_emit: u8,
}

impl MeetingDetectionState {
    pub(crate) fn update(
        &mut self,
        active_external_input: bool,
        os_scribe_capture_active: bool,
    ) -> Option<MeetingDetectionEvent> {
        let should_be_active = active_external_input && !os_scribe_capture_active;
        if should_be_active {
            self.inactive_polls = 0;
            if !self.active {
                self.active = true;
                self.active_polls_since_emit = 0;
                return Some(MeetingDetectionEvent::Detected);
            }

            self.active_polls_since_emit = self.active_polls_since_emit.saturating_add(1);
            if self.active_polls_since_emit >= HEARTBEAT_EVERY_ACTIVE_POLLS {
                self.active_polls_since_emit = 0;
                return Some(MeetingDetectionEvent::Detected);
            }
            return None;
        }

        self.active_polls_since_emit = 0;
        if !self.active {
            self.inactive_polls = 0;
            return None;
        }

        self.inactive_polls = self.inactive_polls.saturating_add(1);
        if self.inactive_polls >= CLEAR_AFTER_INACTIVE_POLLS {
            self.active = false;
            self.inactive_polls = 0;
            return Some(MeetingDetectionEvent::Cleared);
        }

        None
    }
}

pub(crate) fn active_external_pids(
    active_input_pids: &[u32],
    owned_pids: &BTreeSet<u32>,
) -> Vec<u32> {
    active_input_pids
        .iter()
        .copied()
        .filter(|pid| *pid != 0 && !owned_pids.contains(pid))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_external_pids_excludes_owned_processes() {
        let owned = BTreeSet::from([10, 20]);

        assert_eq!(active_external_pids(&[0, 10, 30, 20, 40], &owned), vec![
            30, 40
        ]);
    }

    #[test]
    fn detector_shows_when_external_input_starts() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );
        assert_eq!(state.update(true, false), None);
    }

    #[test]
    fn detector_suppresses_while_os_scribe_capture_is_active() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(state.update(true, true), None);
        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );
    }

    #[test]
    fn detector_clears_after_inactive_debounce() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        assert_eq!(state.update(false, false), None);
        assert_eq!(
            state.update(false, false),
            Some(MeetingDetectionEvent::Cleared)
        );
        assert_eq!(state.update(false, false), None);
    }

    #[test]
    fn detector_emits_heartbeat_while_active() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        for _ in 0..(HEARTBEAT_EVERY_ACTIVE_POLLS - 1) {
            assert_eq!(state.update(true, false), None);
        }
        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );
    }

    #[test]
    fn detector_clears_when_os_scribe_capture_starts() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        assert_eq!(state.update(true, true), None);
        assert_eq!(
            state.update(true, true),
            Some(MeetingDetectionEvent::Cleared)
        );
    }
}
