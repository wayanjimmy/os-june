use serde::Serialize;
use std::{collections::BTreeSet, thread, time::Duration};
use tauri::{AppHandle, Emitter};

const CLEAR_AFTER_INACTIVE_POLLS: u8 = 2;
const HEARTBEAT_EVERY_ACTIVE_POLLS: u8 = 5;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MEETING_DETECTION_EVENT_NAME: &str = "meeting-detection-event";

pub fn setup(app: &mut tauri::App) {
    #[cfg(target_os = "macos")]
    spawn_monitor(app.handle().clone());

    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

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

#[cfg(target_os = "macos")]
fn spawn_monitor(app: AppHandle) {
    thread::spawn(move || {
        let mut state = MeetingDetectionState::default();
        let mut warned_after_probe_error = false;

        loop {
            thread::sleep(POLL_INTERVAL);

            let active_pids = match active_input_process_pids() {
                Ok(active_pids) => {
                    warned_after_probe_error = false;
                    active_pids
                }
                Err(error) => {
                    if !warned_after_probe_error {
                        tracing::warn!(%error, "meeting detection probe failed");
                        warned_after_probe_error = true;
                    }
                    Vec::new()
                }
            };
            let external_pids = active_external_pids(&active_pids, &owned_pids(&app));
            let capture_active = crate::audio::capture::is_capture_active();
            if let Some(event) = state.update(!external_pids.is_empty(), capture_active) {
                emit_detection_event(&app, event, external_pids.len());
            }
        }
    });
}

fn owned_pids(app: &AppHandle) -> BTreeSet<u32> {
    let mut pids = BTreeSet::from([std::process::id()]);
    if let Some(helper_pid) = crate::dictation::dictation_helper_pid(app) {
        pids.insert(helper_pid);
    }
    pids
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingDetectionPayload {
    active_process_count: usize,
}

#[derive(Debug, Serialize)]
struct MeetingDetectionEnvelope {
    #[serde(rename = "type")]
    event_type: &'static str,
    payload: MeetingDetectionPayload,
}

fn emit_detection_event(
    app: &AppHandle,
    event: MeetingDetectionEvent,
    active_process_count: usize,
) {
    let event_type = match event {
        MeetingDetectionEvent::Detected => {
            crate::dictation::show_hud_window(app);
            "meeting_detected"
        }
        MeetingDetectionEvent::Cleared => "meeting_cleared",
    };
    let payload = MeetingDetectionEnvelope {
        event_type,
        payload: MeetingDetectionPayload {
            active_process_count,
        },
    };
    match serde_json::to_string(&payload) {
        Ok(payload) => {
            let _ = app.emit(MEETING_DETECTION_EVENT_NAME, payload);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to encode meeting detection event");
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::active_input_process_pids;

#[cfg(not(target_os = "macos"))]
pub(crate) fn active_input_process_pids() -> Result<Vec<u32>, ProbeError> {
    Ok(Vec::new())
}

#[derive(Debug)]
pub(crate) struct ProbeError {
    operation: &'static str,
    status: i32,
}

impl ProbeError {
    fn new(operation: &'static str, status: i32) -> Self {
        Self { operation, status }
    }
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} failed with OSStatus {}",
            self.operation, self.status
        )
    }
}

impl std::error::Error for ProbeError {}

#[cfg(target_os = "macos")]
mod macos {
    use super::ProbeError;
    use std::{ffi::c_void, mem, ptr};

    type AudioObjectId = u32;
    type AudioObjectPropertySelector = u32;
    type AudioObjectPropertyScope = u32;
    type AudioObjectPropertyElement = u32;
    type OsStatus = i32;

    const AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectId = 1;
    const AUDIO_OBJECT_UNKNOWN: AudioObjectId = 0;
    const AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: AudioObjectPropertyScope = four_cc(*b"glob");
    const AUDIO_OBJECT_PROPERTY_SCOPE_INPUT: AudioObjectPropertyScope = four_cc(*b"inpt");
    const AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: AudioObjectPropertyElement = 0;
    const AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: AudioObjectPropertySelector =
        four_cc(*b"prs#");
    const AUDIO_PROCESS_PROPERTY_PID: AudioObjectPropertySelector = four_cc(*b"ppid");
    const AUDIO_PROCESS_PROPERTY_BUNDLE_ID: AudioObjectPropertySelector = four_cc(*b"pbid");
    const AUDIO_PROCESS_PROPERTY_DEVICES: AudioObjectPropertySelector = four_cc(*b"pdv#");
    const AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT: AudioObjectPropertySelector = four_cc(*b"piri");

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
        element: AudioObjectPropertyElement,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object_id: AudioObjectId,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
        ) -> OsStatus;

        fn AudioObjectGetPropertyData(
            object_id: AudioObjectId,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> OsStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut i8,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;

        fn CFRelease(cf: *const c_void);
    }

    pub(crate) fn active_input_process_pids() -> Result<Vec<u32>, ProbeError> {
        let mut pids = Vec::new();
        for process_object in process_objects()? {
            if process_object == AUDIO_OBJECT_UNKNOWN {
                continue;
            }
            let running_input = read_u32_property(
                process_object,
                AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT,
                "read process input state",
            )
            .unwrap_or_default();
            if running_input == 0 {
                continue;
            }
            if !process_has_input_devices(process_object)
                || read_process_bundle_id(process_object)
                    .ok()
                    .flatten()
                    .is_none()
            {
                continue;
            }
            if let Ok(Some(pid)) = read_process_pid(process_object) {
                pids.push(pid);
            }
        }
        pids.sort_unstable();
        pids.dedup();
        Ok(pids)
    }

    fn process_objects() -> Result<Vec<AudioObjectId>, ProbeError> {
        read_object_array_property(
            AUDIO_OBJECT_SYSTEM_OBJECT,
            AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST,
            AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
            "read process object list size",
            "read process object list",
        )
    }

    fn read_object_array_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
        size_operation: &'static str,
        data_operation: &'static str,
    ) -> Result<Vec<AudioObjectId>, ProbeError> {
        let address = property_address_with_scope(selector, scope);
        let mut data_size = 0_u32;
        status_result(size_operation, unsafe {
            AudioObjectGetPropertyDataSize(object_id, &address, 0, ptr::null(), &mut data_size)
        })?;

        if data_size == 0 {
            return Ok(Vec::new());
        }

        let object_count = data_size as usize / mem::size_of::<AudioObjectId>();
        let mut objects = vec![AUDIO_OBJECT_UNKNOWN; object_count];
        status_result(data_operation, unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                ptr::null(),
                &mut data_size,
                objects.as_mut_ptr().cast(),
            )
        })?;

        let actual_count = data_size as usize / mem::size_of::<AudioObjectId>();
        objects.truncate(actual_count);
        Ok(objects)
    }

    fn process_devices(
        process_object: AudioObjectId,
        scope: AudioObjectPropertyScope,
    ) -> Result<Vec<AudioObjectId>, ProbeError> {
        read_object_array_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_DEVICES,
            scope,
            "read process device list size",
            "read process device list",
        )
    }

    fn read_process_pid(process_object: AudioObjectId) -> Result<Option<u32>, ProbeError> {
        let pid = read_i32_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_PID,
            "read process pid",
        )?;
        if pid <= 0 {
            Ok(None)
        } else {
            Ok(Some(pid as u32))
        }
    }

    fn process_has_input_devices(process_object: AudioObjectId) -> bool {
        process_devices(process_object, AUDIO_OBJECT_PROPERTY_SCOPE_INPUT)
            .map(|devices| !devices.is_empty())
            .unwrap_or(false)
    }

    fn read_process_bundle_id(process_object: AudioObjectId) -> Result<Option<String>, ProbeError> {
        let mut value: *const c_void = ptr::null();
        read_scalar_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_BUNDLE_ID,
            "read process bundle id",
            &mut value,
        )?;
        if value.is_null() {
            return Ok(None);
        }

        let mut buffer = vec![0_i8; 512];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as isize,
                0x0800_0100,
            )
        };
        unsafe {
            CFRelease(value);
        }
        if ok == 0 {
            return Ok(None);
        }
        let value = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        Ok((!value.is_empty()).then_some(value))
    }

    fn read_i32_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
    ) -> Result<i32, ProbeError> {
        let mut value = 0_i32;
        read_scalar_property(object_id, selector, operation, &mut value)?;
        Ok(value)
    }

    fn read_u32_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
    ) -> Result<u32, ProbeError> {
        let mut value = 0_u32;
        read_scalar_property(object_id, selector, operation, &mut value)?;
        Ok(value)
    }

    fn read_scalar_property<T>(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
        value: &mut T,
    ) -> Result<(), ProbeError> {
        let address = property_address(selector);
        let mut data_size = mem::size_of::<T>() as u32;
        status_result(operation, unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                ptr::null(),
                &mut data_size,
                (value as *mut T).cast(),
            )
        })
    }

    fn property_address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        property_address_with_scope(selector, AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL)
    }

    fn property_address_with_scope(
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            selector,
            scope,
            element: AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
        }
    }

    fn status_result(operation: &'static str, status: OsStatus) -> Result<(), ProbeError> {
        if status == 0 {
            Ok(())
        } else {
            Err(ProbeError::new(operation, status))
        }
    }

    const fn four_cc(value: [u8; 4]) -> u32 {
        ((value[0] as u32) << 24)
            | ((value[1] as u32) << 16)
            | ((value[2] as u32) << 8)
            | value[3] as u32
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn four_cc_matches_core_audio_constants() {
            assert_eq!(AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST, 0x7072_7323);
            assert_eq!(AUDIO_PROCESS_PROPERTY_PID, 0x7070_6964);
            assert_eq!(AUDIO_PROCESS_PROPERTY_BUNDLE_ID, 0x7062_6964);
            assert_eq!(AUDIO_PROCESS_PROPERTY_DEVICES, 0x7064_7623);
            assert_eq!(AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT, 0x7069_7269);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_external_pids_excludes_owned_processes() {
        let owned = BTreeSet::from([10, 20]);

        assert_eq!(
            active_external_pids(&[0, 10, 30, 20, 40], &owned),
            vec![30, 40]
        );
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
