use crate::domain::types::AppError;
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    ffi::c_void,
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    ptr,
    sync::Mutex,
    thread,
};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct HelperProcess {
    child: Child,
    stdin: ChildStdin,
}

pub struct HelperState {
    process: Mutex<Option<HelperProcess>>,
}

pub struct LastDictationEvent {
    event: Mutex<Option<String>>,
}

pub struct DictationSettingsState {
    path: PathBuf,
    settings: Mutex<DictationSettings>,
}

pub struct HotkeyStatus {
    event: Mutex<serde_json::Value>,
}

#[cfg(target_os = "macos")]
pub struct HotkeyManager {
    hotkeys: Mutex<Option<carbon_hotkeys::CarbonHotkeys>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSettings {
    pub shortcut: DictationShortcutSetting,
    pub microphone: DictationMicrophoneSetting,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            shortcut: DictationShortcutSetting::fn_space(),
            microphone: DictationMicrophoneSetting::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutSetting {
    pub key_code: u32,
    pub code: String,
    pub modifiers: DictationShortcutModifiers,
    pub label: String,
}

impl<'de> Deserialize<'de> for DictationShortcutSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ShortcutSettingValue {
            key_code: u32,
            code: String,
            modifiers: DictationShortcutModifiers,
            label: String,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(shortcut) = value.as_str() {
            return DictationShortcutPreset::from_id(shortcut)
                .map(DictationShortcutSetting::from)
                .ok_or_else(|| serde::de::Error::custom("unknown shortcut preset"));
        }

        let shortcut: ShortcutSettingValue =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            key_code: shortcut.key_code,
            code: shortcut.code,
            modifiers: shortcut.modifiers,
            label: shortcut.label,
        })
    }
}

impl DictationShortcutSetting {
    fn fn_space() -> Self {
        DictationShortcutPreset::FnSpace.into()
    }

    fn control_option_space() -> Self {
        DictationShortcutPreset::ControlOptionSpace.into()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutModifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub function: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutInput {
    pub code: String,
    pub modifiers: DictationShortcutModifiers,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DictationShortcutPreset {
    FnSpace,
    ControlOptionSpace,
}

impl DictationShortcutPreset {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "fn_space" => Some(Self::FnSpace),
            "control_option_space" => Some(Self::ControlOptionSpace),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::FnSpace => "Fn+Space",
            Self::ControlOptionSpace => "Ctrl+Opt+Space",
        }
    }
}

impl From<DictationShortcutPreset> for DictationShortcutSetting {
    fn from(shortcut: DictationShortcutPreset) -> Self {
        match shortcut {
            DictationShortcutPreset::FnSpace => Self {
                key_code: 0x31,
                code: "Space".to_string(),
                modifiers: DictationShortcutModifiers {
                    function: true,
                    ..DictationShortcutModifiers::default()
                },
                label: shortcut.label().to_string(),
            },
            DictationShortcutPreset::ControlOptionSpace => Self {
                key_code: 0x31,
                code: "Space".to_string(),
                modifiers: DictationShortcutModifiers {
                    control: true,
                    option: true,
                    ..DictationShortcutModifiers::default()
                },
                label: shortcut.label().to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationMicrophoneSetting {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSettingsResponse {
    pub settings: DictationSettings,
}

impl DictationShortcutInput {
    fn into_setting(self) -> Result<DictationShortcutSetting, AppError> {
        if self.code.trim().is_empty() || self.label.trim().is_empty() {
            return Err(AppError::new(
                "dictation_shortcut_incomplete",
                "Shortcut is incomplete.",
            ));
        }

        let key_code = key_code_for_code(&self.code).ok_or_else(|| {
            AppError::new(
                "dictation_shortcut_unsupported",
                "Shortcut key is not supported.",
            )
        })?;

        let has_modifier = self.modifiers.command
            || self.modifiers.control
            || self.modifiers.option
            || self.modifiers.shift
            || self.modifiers.function;
        if !has_modifier {
            return Err(AppError::new(
                "dictation_shortcut_modifier_required",
                "Shortcut must include at least one modifier key.",
            ));
        }

        Ok(DictationShortcutSetting {
            key_code,
            code: self.code,
            modifiers: self.modifiers,
            label: self.label,
        })
    }
}

pub fn setup(app: &mut tauri::App) {
    manage_settings(app.handle());
    app.manage(LastDictationEvent {
        event: Mutex::new(None),
    });

    let helper = spawn_helper(app.handle()).ok();
    app.manage(HelperState {
        process: Mutex::new(helper),
    });

    {
        let settings = app.state::<DictationSettingsState>();
        let helper_state = app.state::<HelperState>();
        let microphone = settings
            .settings
            .lock()
            .ok()
            .map(|settings| settings.microphone.clone());
        if let Some(microphone) = microphone {
            let _ = apply_microphone_setting(&helper_state, &microphone);
        }
    }

    #[cfg(target_os = "macos")]
    {
        app.manage(HotkeyManager {
            hotkeys: Mutex::new(None),
        });

        let settings = app.state::<DictationSettingsState>();
        let shortcut = settings
            .settings
            .lock()
            .map(|settings| settings.shortcut.clone())
            .unwrap_or_else(|_| DictationShortcutSetting::fn_space());
        let hotkeys = carbon_hotkeys::register(app.handle(), shortcut.clone()).or_else(|error| {
            if shortcut != DictationShortcutSetting::fn_space() {
                return Err(error);
            }

            carbon_hotkeys::register(app.handle(), DictationShortcutSetting::control_option_space())
                .map(|hotkeys| {
                    if let Err(settings_error) = update_settings(&settings, |settings| {
                        settings.shortcut = DictationShortcutSetting::control_option_space();
                    }) {
                        let message = format!(
                            "Could not save fallback shortcut: {}",
                            settings_error.message
                        );
                        eprintln!("{message}");
                        let _ = app.emit(
                            "dictation-event",
                            hotkey_error_event(message).to_string(),
                        );
                    }
                    hotkeys
                })
                .map_err(|fallback_error| format!("{error} {fallback_error}"))
        });

        let event = match hotkeys {
            Ok(hotkeys) => {
                let shortcut = hotkeys.shortcut().clone();
                if let Some(manager) = app.try_state::<HotkeyManager>() {
                    if let Ok(mut active_hotkeys) = manager.hotkeys.lock() {
                        *active_hotkeys = Some(hotkeys);
                    }
                }
                hotkey_ready_event(&shortcut)
            }
            Err(error) => hotkey_error_event(error),
        };

        app.manage(HotkeyStatus {
            event: Mutex::new(event.clone()),
        });
        let _ = app.emit("dictation-event", event.to_string());
    }
}

#[tauri::command]
pub fn dictation_settings(
    state: State<'_, DictationSettingsState>,
) -> Result<DictationSettingsResponse, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?
        .clone();
    Ok(DictationSettingsResponse { settings })
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub fn set_dictation_shortcut(
    app: AppHandle,
    state: State<'_, DictationSettingsState>,
    hotkey_manager: State<'_, HotkeyManager>,
    hotkey_status: State<'_, HotkeyStatus>,
    shortcut: DictationShortcutInput,
) -> Result<DictationSettings, AppError> {
    let shortcut = shortcut.into_setting()?;
    let current_settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?
        .clone();

    if current_settings.shortcut == shortcut {
        set_hotkey_status(&hotkey_status, hotkey_ready_event(&shortcut));
        return Ok(current_settings);
    }

    let next_hotkeys = carbon_hotkeys::register(&app, shortcut.clone()).map_err(|error| {
        AppError::new("dictation_hotkey_registration_failed", error.to_string())
    })?;
    let settings = update_settings(&state, |settings| {
        settings.shortcut = shortcut;
    })?;

    {
        let mut active_hotkeys = hotkey_manager
            .hotkeys
            .lock()
            .map_err(|_| AppError::new("dictation_hotkey_unavailable", "Hotkey lock failed."))?;
        *active_hotkeys = Some(next_hotkeys);
    }

    set_hotkey_status(&hotkey_status, hotkey_ready_event(&settings.shortcut));
    Ok(settings)
}

#[tauri::command]
pub fn set_dictation_microphone(
    state: State<'_, DictationSettingsState>,
    helper_state: State<'_, HelperState>,
    id: Option<String>,
    name: Option<String>,
) -> Result<DictationSettings, AppError> {
    let settings = update_settings(&state, |settings| {
        settings.microphone = DictationMicrophoneSetting { id, name };
    })?;
    apply_microphone_setting(&helper_state, &settings.microphone)?;
    Ok(settings)
}

#[tauri::command]
pub fn dictation_helper_command(
    state: State<'_, HelperState>,
    command: serde_json::Value,
) -> Result<(), AppError> {
    send_helper_command(&state, command)
}

#[tauri::command]
pub fn dictation_hotkey_status(state: State<'_, HotkeyStatus>) -> serde_json::Value {
    state
        .event
        .lock()
        .map(|event| event.clone())
        .unwrap_or_else(|_| hotkey_error_event("Hotkey status lock was poisoned.".to_string()))
}

#[tauri::command]
pub fn latest_dictation_event(state: State<'_, LastDictationEvent>) -> Option<String> {
    state.event.lock().ok().and_then(|event| event.clone())
}

pub fn stop_helper(app: &AppHandle) {
    let state = app.state::<HelperState>();
    let Ok(mut guard) = state.process.lock() else {
        return;
    };
    let Some(mut process) = guard.take() else {
        return;
    };

    let _ = process.stdin.write_all(b"{\"type\":\"shutdown\"}\n");
    let _ = process.stdin.flush();
    let _ = process.child.kill();
    let _ = process.child.wait();
}

fn send_helper_command(state: &HelperState, command: serde_json::Value) -> Result<(), AppError> {
    let mut guard = state
        .process
        .lock()
        .map_err(|_| AppError::new("dictation_helper_unavailable", "Helper lock failed."))?;
    let process = guard.as_mut().ok_or_else(|| {
        AppError::new(
            "dictation_helper_unavailable",
            "Dictation helper process is not running.",
        )
    })?;
    let mut line = serde_json::to_string(&command)
        .map_err(|error| AppError::new("dictation_helper_command_invalid", error.to_string()))?;
    line.push('\n');
    process
        .stdin
        .write_all(line.as_bytes())
        .map_err(|error| AppError::new("dictation_helper_write_failed", error.to_string()))?;
    process
        .stdin
        .flush()
        .map_err(|error| AppError::new("dictation_helper_write_failed", error.to_string()))
}

fn apply_microphone_setting(
    helper_state: &HelperState,
    microphone: &DictationMicrophoneSetting,
) -> Result<(), AppError> {
    send_helper_command(
        helper_state,
        serde_json::json!({
            "type": "set_microphone",
            "id": microphone.id,
            "name": microphone.name,
        }),
    )
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join("dictation-settings.json"))
}

fn manage_settings(app: &AppHandle) {
    let path = settings_path(app).unwrap_or_else(|| PathBuf::from("dictation-settings.json"));
    let settings = load_settings(app);
    app.manage(DictationSettingsState {
        path,
        settings: Mutex::new(settings),
    });
}

fn load_settings(app: &AppHandle) -> DictationSettings {
    let Some(path) = settings_path(app) else {
        return DictationSettings::default();
    };

    fs::read_to_string(path)
        .ok()
        .and_then(|settings| serde_json::from_str::<DictationSettings>(&settings).ok())
        .unwrap_or_default()
}

fn update_settings(
    state: &DictationSettingsState,
    update: impl FnOnce(&mut DictationSettings),
) -> Result<DictationSettings, AppError> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?;
    update(&mut settings);
    save_settings(state, &settings)?;
    Ok(settings.clone())
}

fn save_settings(
    state: &DictationSettingsState,
    settings: &DictationSettings,
) -> Result<(), AppError> {
    if let Some(parent) = state.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))?;
    }

    let serialized = serde_json::to_string_pretty(settings)
        .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))?;
    fs::write(&state.path, serialized)
        .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))
}

fn helper_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        if let Some(repo_dir) = manifest_dir.parent() {
            paths.push(
                repo_dir
                    .join(".tauri-helper")
                    .join("OS Scribe Dictation Helper.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("os-scribe-dictation-helper"),
            );
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            paths.push(exe_dir.join("os-scribe-dictation-helper"));
            paths.push(exe_dir.join("../Resources/os-scribe-dictation-helper"));
        }
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        paths.push(resource_dir.join("os-scribe-dictation-helper"));
        paths.push(
            resource_dir
                .join("native")
                .join("bin")
                .join("OS Scribe Dictation Helper.app")
                .join("Contents")
                .join("MacOS")
                .join("os-scribe-dictation-helper"),
        );
        paths.push(
            resource_dir
                .join("OS Scribe Dictation Helper.app")
                .join("Contents")
                .join("MacOS")
                .join("os-scribe-dictation-helper"),
        );
    }

    paths
}

fn spawn_helper(app: &AppHandle) -> Result<HelperProcess, AppError> {
    let helper_path = helper_candidates(app)
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            AppError::new(
                "dictation_helper_missing",
                "Could not find bundled dictation helper binary.",
            )
        })?;

    let mut child = Command::new(&helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::new(
                "dictation_helper_start_failed",
                format!("Failed to start helper at {}: {error}", helper_path.display()),
            )
        })?;

    let stdin = child.stdin.take().ok_or_else(|| {
        AppError::new("dictation_helper_start_failed", "Helper stdin was unavailable.")
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::new(
            "dictation_helper_start_failed",
            "Helper stdout was unavailable.",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AppError::new(
            "dictation_helper_start_failed",
            "Helper stderr was unavailable.",
        )
    })?;

    let output_app = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            handle_helper_event_line(&output_app, line);
        }
    });

    let error_app = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = error_app.emit("dictation-helper-stderr", line);
        }
    });

    Ok(HelperProcess { child, stdin })
}

fn handle_helper_event_line(app: &AppHandle, line: String) {
    let event = serde_json::from_str::<serde_json::Value>(&line).ok();
    let event_type = event
        .as_ref()
        .and_then(|event| event.get("type"))
        .and_then(serde_json::Value::as_str);

    if let Some(state) = app.try_state::<LastDictationEvent>() {
        if let Ok(mut event) = state.event.lock() {
            match dictation_event_visibility(event_type) {
                DictationEventVisibility::Show => *event = Some(line.clone()),
                DictationEventVisibility::Hide => *event = None,
                DictationEventVisibility::Ignore => {}
            }
        }
    }

    let _ = app.emit("dictation-event", line);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DictationEventVisibility {
    Show,
    Hide,
    Ignore,
}

fn dictation_event_visibility(event_type: Option<&str>) -> DictationEventVisibility {
    match event_type {
        Some(
            "listening_started"
            | "audio_level"
            | "finalizing_transcript"
            | "final_transcript"
            | "paste_target",
        ) => DictationEventVisibility::Show,
        Some("paste_completed" | "error" | "shutdown_ack") => DictationEventVisibility::Hide,
        _ => DictationEventVisibility::Ignore,
    }
}

fn hotkey_ready_event(shortcut: &DictationShortcutSetting) -> serde_json::Value {
    serde_json::json!({
        "type": "hotkey_trigger_ready",
        "payload": {
            "shortcut": shortcut.label,
            "shortcuts": shortcut.label,
        },
    })
}

fn hotkey_error_event(error: String) -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "payload": {
            "code": "hotkey_registration_failed",
            "message": error,
        },
    })
}

fn set_hotkey_status(state: &HotkeyStatus, event: serde_json::Value) {
    if let Ok(mut status) = state.event.lock() {
        *status = event;
    }
}

pub fn key_code_for_code(code: &str) -> Option<u32> {
    Some(match code {
        "KeyA" => 0x00,
        "KeyS" => 0x01,
        "KeyD" => 0x02,
        "KeyF" => 0x03,
        "KeyH" => 0x04,
        "KeyG" => 0x05,
        "KeyZ" => 0x06,
        "KeyX" => 0x07,
        "KeyC" => 0x08,
        "KeyV" => 0x09,
        "KeyB" => 0x0b,
        "KeyQ" => 0x0c,
        "KeyW" => 0x0d,
        "KeyE" => 0x0e,
        "KeyR" => 0x0f,
        "KeyY" => 0x10,
        "KeyT" => 0x11,
        "Digit1" => 0x12,
        "Digit2" => 0x13,
        "Digit3" => 0x14,
        "Digit4" => 0x15,
        "Digit6" => 0x16,
        "Digit5" => 0x17,
        "Equal" => 0x18,
        "Digit9" => 0x19,
        "Digit7" => 0x1a,
        "Minus" => 0x1b,
        "Digit8" => 0x1c,
        "Digit0" => 0x1d,
        "BracketRight" => 0x1e,
        "KeyO" => 0x1f,
        "KeyU" => 0x20,
        "BracketLeft" => 0x21,
        "KeyI" => 0x22,
        "KeyP" => 0x23,
        "Enter" => 0x24,
        "KeyL" => 0x25,
        "KeyJ" => 0x26,
        "Quote" => 0x27,
        "KeyK" => 0x28,
        "Semicolon" => 0x29,
        "Backslash" => 0x2a,
        "Comma" => 0x2b,
        "Slash" => 0x2c,
        "KeyN" => 0x2d,
        "KeyM" => 0x2e,
        "Period" => 0x2f,
        "Tab" => 0x30,
        "Space" => 0x31,
        "Backquote" => 0x32,
        "Backspace" => 0x33,
        "Escape" => 0x35,
        "F1" => 0x7a,
        "ArrowLeft" => 0x7b,
        "ArrowRight" => 0x7c,
        "ArrowDown" => 0x7d,
        "ArrowUp" => 0x7e,
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
mod carbon_hotkeys {
    use super::*;

    type OSStatus = i32;
    type OSType = u32;
    type EventHandlerCallRef = *mut c_void;
    type EventRef = *mut c_void;
    type EventHandlerRef = *mut c_void;
    type EventHotKeyRef = *mut c_void;
    type EventTargetRef = *mut c_void;
    type EventHandlerUPP = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

    const NO_ERR: OSStatus = 0;
    const K_EVENT_CLASS_KEYBOARD: OSType = four_char_code(*b"keyb");
    const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
    const K_EVENT_PARAM_DIRECT_OBJECT: OSType = four_char_code(*b"----");
    const TYPE_EVENT_HOT_KEY_ID: OSType = four_char_code(*b"hkid");
    const COMMAND_KEY: u32 = 1 << 8;
    const SHIFT_KEY: u32 = 1 << 9;
    const OPTION_KEY: u32 = 1 << 11;
    const CONTROL_KEY: u32 = 1 << 12;
    const FN_KEY: u32 = 1 << 17;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventTypeSpec {
        event_class: OSType,
        event_kind: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct EventHotKeyID {
        signature: OSType,
        id: u32,
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn GetApplicationEventTarget() -> EventTargetRef;
        fn InstallEventHandler(
            target: EventTargetRef,
            handler: EventHandlerUPP,
            num_types: u32,
            list: *const EventTypeSpec,
            user_data: *mut c_void,
            out_ref: *mut EventHandlerRef,
        ) -> OSStatus;
        fn RemoveEventHandler(handler_ref: EventHandlerRef) -> OSStatus;
        fn RegisterEventHotKey(
            key_code: u32,
            modifiers: u32,
            hot_key_id: EventHotKeyID,
            target: EventTargetRef,
            options: u32,
            out_ref: *mut EventHotKeyRef,
        ) -> OSStatus;
        fn UnregisterEventHotKey(hot_key_ref: EventHotKeyRef) -> OSStatus;
        fn GetEventParameter(
            event: EventRef,
            name: OSType,
            desired_type: OSType,
            actual_type: *mut OSType,
            buffer_size: usize,
            actual_size: *mut usize,
            out_data: *mut c_void,
        ) -> OSStatus;
    }

    pub struct CarbonHotkeys {
        handler_ref: EventHandlerRef,
        hotkey_refs: Vec<EventHotKeyRef>,
        shortcut: DictationShortcutSetting,
        controller: *mut HotkeyController,
    }

    unsafe impl Send for CarbonHotkeys {}
    unsafe impl Sync for CarbonHotkeys {}

    struct HotkeyController {
        app: AppHandle,
        shortcut_label: String,
    }

    pub fn register(
        app: &AppHandle,
        shortcut: DictationShortcutSetting,
    ) -> Result<CarbonHotkeys, String> {
        let controller = Box::into_raw(Box::new(HotkeyController {
            app: app.clone(),
            shortcut_label: shortcut.label.clone(),
        }));
        let mut handler_ref = ptr::null_mut();
        let event_type = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };

        let handler_status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                hotkey_handler,
                1,
                &event_type,
                controller.cast(),
                &mut handler_ref,
            )
        };

        if handler_status != NO_ERR {
            unsafe {
                drop(Box::from_raw(controller));
            }
            return Err(format!(
                "Could not install Carbon hotkey handler (OSStatus {handler_status})."
            ));
        }

        let mut hotkey_refs = Vec::new();
        if let Err(error) = register_hotkey(&shortcut, &mut hotkey_refs) {
            unsafe {
                RemoveEventHandler(handler_ref);
                drop(Box::from_raw(controller));
            }
            return Err(error);
        }

        Ok(CarbonHotkeys {
            handler_ref,
            hotkey_refs,
            shortcut,
            controller,
        })
    }

    impl CarbonHotkeys {
        pub fn shortcut(&self) -> &DictationShortcutSetting {
            &self.shortcut
        }
    }

    fn register_hotkey(
        shortcut: &DictationShortcutSetting,
        hotkey_refs: &mut Vec<EventHotKeyRef>,
    ) -> Result<(), String> {
        let mut hotkey_ref = ptr::null_mut();
        let status = unsafe {
            RegisterEventHotKey(
                shortcut.key_code,
                modifier_mask(&shortcut.modifiers),
                EventHotKeyID {
                    signature: four_char_code(*b"OSSD"),
                    id: 1,
                },
                GetApplicationEventTarget(),
                0,
                &mut hotkey_ref,
            )
        };

        if status != NO_ERR {
            return Err(format!(
                "Could not register {} (OSStatus {status}).",
                shortcut.label
            ));
        }

        hotkey_refs.push(hotkey_ref);
        Ok(())
    }

    fn modifier_mask(modifiers: &DictationShortcutModifiers) -> u32 {
        let mut mask = 0;
        if modifiers.command {
            mask |= COMMAND_KEY;
        }
        if modifiers.shift {
            mask |= SHIFT_KEY;
        }
        if modifiers.option {
            mask |= OPTION_KEY;
        }
        if modifiers.control {
            mask |= CONTROL_KEY;
        }
        if modifiers.function {
            mask |= FN_KEY;
        }
        mask
    }

    extern "C" fn hotkey_handler(
        _next_handler: EventHandlerCallRef,
        event: EventRef,
        user_data: *mut c_void,
    ) -> OSStatus {
        if event.is_null() || user_data.is_null() {
            return NO_ERR;
        }

        let controller = unsafe { &*(user_data.cast::<HotkeyController>()) };
        if is_registered_hotkey_event(event) {
            let state = controller.app.state::<HelperState>();
            if let Err(error) = send_helper_command(
                &state,
                serde_json::json!({
                    "type": "toggle_listening",
                    "shortcut": controller.shortcut_label,
                }),
            ) {
                let _ = controller.app.emit(
                    "dictation-event",
                    serde_json::json!({
                        "type": "error",
                        "payload": {
                            "code": error.code,
                            "message": error.message,
                        },
                    })
                    .to_string(),
                );
            }
        }

        NO_ERR
    }

    fn is_registered_hotkey_event(event: EventRef) -> bool {
        let mut hotkey_id = EventHotKeyID::default();
        let status = unsafe {
            GetEventParameter(
                event,
                K_EVENT_PARAM_DIRECT_OBJECT,
                TYPE_EVENT_HOT_KEY_ID,
                ptr::null_mut(),
                std::mem::size_of::<EventHotKeyID>(),
                ptr::null_mut(),
                (&mut hotkey_id as *mut EventHotKeyID).cast(),
            )
        };

        if status != NO_ERR {
            return false;
        }

        hotkey_id.id == 1
    }

    impl Drop for CarbonHotkeys {
        fn drop(&mut self) {
            for hotkey_ref in self.hotkey_refs.drain(..) {
                if !hotkey_ref.is_null() {
                    unsafe {
                        UnregisterEventHotKey(hotkey_ref);
                    }
                }
            }

            if !self.handler_ref.is_null() {
                unsafe {
                    RemoveEventHandler(self.handler_ref);
                }
            }

            if !self.controller.is_null() {
                unsafe {
                    drop(Box::from_raw(self.controller));
                }
                self.controller = ptr::null_mut();
            }
        }
    }

    const fn four_char_code(value: [u8; 4]) -> OSType {
        ((value[0] as OSType) << 24)
            | ((value[1] as OSType) << 16)
            | ((value[2] as OSType) << 8)
            | (value[3] as OSType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_fn_space() {
        let settings = DictationSettings::default();

        assert_eq!(settings.shortcut.label, "Fn+Space");
        assert_eq!(settings.shortcut.code, "Space");
        assert!(settings.shortcut.modifiers.function);
    }

    #[test]
    fn deserializes_shortcut_preset() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"shortcut":"control_option_space","microphone":{"id":null,"name":null}}"#,
        )
        .expect("preset should deserialize");

        assert_eq!(settings.shortcut.label, "Ctrl+Opt+Space");
        assert!(settings.shortcut.modifiers.control);
        assert!(settings.shortcut.modifiers.option);
    }

    #[test]
    fn shortcut_input_requires_modifier() {
        let err = DictationShortcutInput {
            code: "KeyT".to_string(),
            modifiers: DictationShortcutModifiers::default(),
            label: "T".to_string(),
        }
        .into_setting()
        .expect_err("shortcut without modifiers should fail");

        assert_eq!(err.code, "dictation_shortcut_modifier_required");
    }

    #[test]
    fn shortcut_input_rejects_unsupported_key() {
        let err = DictationShortcutInput {
            code: "NumpadEnter".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+NumpadEnter".to_string(),
        }
        .into_setting()
        .expect_err("unsupported key should fail");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }
}
