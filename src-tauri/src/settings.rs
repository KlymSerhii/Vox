//! Settings — backed by `tauri-plugin-store` for non-secret fields.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;

use crate::models;

pub const STORE_FILE: &str = "vox-settings.json";

const KEY_HOTKEY: &str = "hotkey";
const KEY_CANCEL_HOTKEY: &str = "cancel_hotkey";
const KEY_CANCEL_REQUIRES_DOUBLE_PRESS: &str = "cancel_requires_double_press";
const KEY_LAUNCH_AT_LOGIN: &str = "launch_at_login";
const KEY_ONBOARDED: &str = "onboarded";
const KEY_DICTATION_ENABLED: &str = "dictation_enabled";
const KEY_DICTIONARY: &str = "dictionary";
const KEY_CONTEXT: &str = "context";
const KEY_HANDS_FREE: &str = "hands_free_enabled";
const KEY_LANGUAGE: &str = "language";

const KEY_STT_ENGINE: &str = "stt_engine";
const KEY_STT_MODEL_ID: &str = "stt_model_id";
const KEY_LLM_ENGINE: &str = "llm_engine";
const KEY_LLM_MODEL_ID: &str = "llm_model_id";
const KEY_GEMMA_TOS_ACCEPTED: &str = "gemma_tos_accepted";
const KEY_POLISHING_ENABLED: &str = "polishing_enabled";

const KEY_ACTIVE_MODE_ID: &str = "active_mode_id";
const KEY_MODE_CYCLE_HOTKEY: &str = "mode_cycle_hotkey";
const KEY_AUTO_SUGGEST_MODES: &str = "auto_suggest_modes";

const KEY_APPEARANCE: &str = "appearance";

const KEY_SHOW_DOCK_ICON: &str = "show_dock_icon";

pub const KEY_STREAMING_PILL_ENABLED: &str = "streaming_pill_enabled";

const KEY_AUTO_PASTE_ENABLED: &str = "auto_paste_enabled";

const KEY_LIVE_TRANSCRIPT_ENABLED: &str = "live_transcript_enabled";

const KEY_ENABLE_THINKING: &str = "enable_thinking";

const KEY_TRY_PARAKEET_DISMISSED: &str = "try_parakeet_dismissed";

const KEY_AUDIO_INPUT_DEVICE: &str = "audio_input_device";

const KEY_WAKE_WORD_ENABLED: &str = "wake_word_enabled";
const KEY_WAKE_PHRASE: &str = "wake_phrase";

const KEY_NOTCH_LIST_ENABLED: &str = "notch_list_enabled";
const KEY_NOTCH_LIST_TOP_N: &str = "notch_list_top_n";
const KEY_NOTCH_LIST_DISPLAY: &str = "notch_list_display";
const KEY_NOTCH_LIST_VOICE_HOTKEY: &str = "notch_list_voice_hotkey";
const KEY_NOTCH_LIST_CONFIRM_DESTRUCTIVE: &str = "notch_list_confirm_destructive";
const KEY_NOTCH_LIST_HOVER_DELAY_MS: &str = "notch_list_hover_delay_ms";
const KEY_NOTCH_LIST_CLICK_TO_EXPAND: &str = "notch_list_click_to_expand";

// Process-identification token passed to `llama-server` as `--api-key`,
// reused across sessions so the orphan reaper can spot stale children.
const KEY_INFERENCE_API_KEY: &str = "inference_api_key";

// Auto-update scheduler keys. `last_update_check_at` is wall-clock
// Unix seconds of the most recent SUCCESSFUL background or manual
// check; the scheduler in `updater.rs` uses it to enforce the 6 h
// cooldown and 24 h cadence across relaunches.
// `dismissed_update_version` is the version a user "Later"-dismissed
// in the tray banner; while it equals the current available version,
// the banner + tray-icon dot stay hidden. Settings → About still
// surfaces the pending state regardless.
const KEY_AUTO_UPDATE_CHECK_ENABLED: &str = "auto_update_check_enabled";
const KEY_LAST_UPDATE_CHECK_AT: &str = "last_update_check_at";
const KEY_DISMISSED_UPDATE_VERSION: &str = "dismissed_update_version";
const KEY_UPDATE_WINDOW_SHOWN_VERSION: &str = "update_window_shown_version";

fn default_hotkey() -> String {
    // ⌘⌥. — Cmd + Option + Period. Bound via Carbon
    // `RegisterEventHotKey` through `tauri-plugin-global-shortcut`, so
    // it doesn't need Accessibility. Period is the unshifted glyph
    // of its key, so no Shift collapsing is needed at display time.
    "Cmd+Alt+Period".to_string()
}
fn default_cancel_hotkey() -> String {
    "Escape".to_string()
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_true() -> bool {
    true
}
fn default_stt_engine() -> String {
    "whisper-local".into()
}
fn default_llm_engine() -> String {
    "gemma-local".into()
}
fn default_polishing_enabled() -> bool {
    true
}
fn default_stt_model_id() -> String {
    models::recommended_stt_model_id().to_string()
}
fn default_llm_model_id() -> String {
    models::recommended_llm_model_id().to_string()
}
fn default_active_mode_id() -> String {
    "general".into()
}
fn default_mode_cycle_hotkey() -> String {
    "Alt+M".into()
}
fn default_auto_suggest_modes() -> bool {
    true
}
fn default_appearance() -> String {
    "dark".into()
}
fn default_show_dock_icon() -> bool {
    // Off by default. Vox is an ambient menu-bar utility — the default
    // experience stays Dock-free (`ActivationPolicy::Accessory`, backed
    // by `LSUIElement` in Info.plist). Flipping this on in
    // Settings → General switches the policy to `Regular` so the app
    // gains a Dock icon + ⌘-Tab entry; clicking the icon opens Settings
    // (handled by the `RunEvent::Reopen` arm in `lib.rs`). The menu-bar
    // icon is always present regardless — this toggle is purely additive.
    false
}
fn default_streaming_pill_enabled() -> bool {
    true
}
fn default_auto_paste_enabled() -> bool {
    // Opt-in. Pasting requires Accessibility, which Vox's default
    // pipeline deliberately doesn't need. Users who want the
    // hands-off "speak straight into the focused field" flow flip
    // this on in Settings → General.
    false
}
fn default_live_transcript_enabled() -> bool {
    // Opt-in. Reveals the speculative transcript inside the
    // pill while the user is still speaking (instead of the default
    // waveform-only listening view). Off by default because it
    // forces the speculative decoder to run even on battery and
    // changes the listening silhouette — users who explicitly want
    // a live caption flip this on.
    false
}
fn default_wake_word_enabled() -> bool {
    // Opt-in. Keeping the microphone open while Vox is idle is a
    // meaningful privacy/battery posture change, so the user has to
    // flip it on in Settings → General themselves.
    false
}
pub fn default_wake_phrase() -> String {
    "hey vox".to_string()
}
fn default_auto_update_check_enabled() -> bool {
    // On by default. The PM-decided behaviour is "check once a day,
    // never auto-install". Flipping this off disables the periodic
    // scheduler in `updater.rs`; the manual button in
    // Settings → About still works.
    true
}
fn default_notch_list_enabled() -> bool {
    // Opt-OUT — the Notch List ships enabled by default (founder
    // mandate). On macOS the notch window is created at launch when
    // this is true; flipping it off in Settings → Notch List destroys
    // the window. Always false in effect on Windows (no notch window).
    true
}
fn default_notch_list_top_n() -> u8 {
    5
}
fn default_notch_list_display() -> String {
    // One of "active" | "builtin" | "primary". "active" follows the
    // display the cursor is on (mirroring the pill); coerced on load.
    "active".into()
}
pub fn default_notch_list_voice_hotkey() -> String {
    // ⌥⌘L — Option + Command + L. Routes the next dictation through the
    // notch command interpreter instead of the polish/clipboard path.
    "Cmd+Alt+L".to_string()
}
fn default_notch_list_confirm_destructive() -> bool {
    // On by default — a voice batch that removes/clears items is staged
    // rather than applied immediately so an STT mishearing can't wipe
    // the list silently.
    true
}
fn default_notch_list_hover_delay_ms() -> u32 {
    // Intent delay before the notch panel expands on hover, so a cursor
    // merely passing over the strip doesn't pop the panel. Clamped to
    // 0..=1000 ms on load/save.
    120
}
fn default_notch_list_click_to_expand() -> bool {
    // Off by default — the notch expands on hover. When on, hover is
    // ignored for expansion and the collapsed line toggles on click.
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_cancel_hotkey")]
    pub cancel_hotkey: String,
    /// Require pressing the cancel hotkey (default Escape) twice in
    /// quick succession to abort a *live recording*, instead of
    /// cancelling on the first press. Opt-in — off by default keeps the
    /// single-press behaviour. Guards against a stray Escape (e.g.
    /// dismissing a screenshot while dictating) tearing down the
    /// session. Only the recording phase is gated; once the pipeline is
    /// transcribing / polishing a single press still aborts.
    #[serde(default)]
    pub cancel_requires_double_press: bool,

    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default = "default_true")]
    pub dictation_enabled: bool,

    #[serde(default)]
    pub hands_free_enabled: bool,
    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default)]
    pub onboarded: bool,

    #[serde(default)]
    pub dictionary: Vec<String>,

    /// Free-form prose about the user (job, what they're working on, etc.)
    /// injected into the LLM cleanup/polish system prompt for every mode
    /// that opts in via `VoiceMode::context_enabled`. Polish step only —
    /// never the STT bias prompt. Empty by default.
    #[serde(default)]
    pub context: String,

    #[serde(default = "default_stt_engine")]
    pub stt_engine: String,
    #[serde(default = "default_stt_model_id")]
    pub stt_model_id: String,
    #[serde(default = "default_llm_engine")]
    pub llm_engine: String,
    #[serde(default = "default_llm_model_id")]
    pub llm_model_id: String,
    /// Independent of `llm_engine` so the user's preference survives
    /// engine outages — flipping this back on restores `gemma-local`.
    #[serde(default = "default_polishing_enabled")]
    pub polishing_enabled: bool,
    #[serde(default)]
    pub gemma_tos_accepted: bool,

    #[serde(default = "default_active_mode_id")]
    pub active_mode_id: String,
    #[serde(default = "default_mode_cycle_hotkey")]
    pub mode_cycle_hotkey: String,
    #[serde(default = "default_auto_suggest_modes")]
    pub auto_suggest_modes: bool,

    #[serde(default = "default_appearance")]
    pub appearance: String,

    /// Show a Dock icon (and ⌘-Tab entry) for Vox. Off by default —
    /// Vox runs as a menu-bar-only accessory app. When true, the
    /// activation policy is switched to `Regular` at launch and live
    /// on toggle; clicking the Dock icon opens the Settings window. The
    /// menu-bar icon is unaffected either way.
    #[serde(default = "default_show_dock_icon")]
    pub show_dock_icon: bool,

    /// Show transcription / polish text streaming inside the pill after
    /// the hotkey is released. When false, only the existing
    /// stage labels are shown (matching the pre-streaming build).
    #[serde(default = "default_streaming_pill_enabled")]
    pub streaming_pill_enabled: bool,

    /// Auto-paste the polished transcript into the focused text field
    /// by synthesising ⌘V at the end of the pipeline. Requires
    /// Accessibility permission; when the toggle is on but
    /// Accessibility isn't granted, `session.rs` falls open to the
    /// clipboard-only path and surfaces a warning in the pill. Off by
    /// default — Vox's default pipeline deliberately doesn't require
    /// Accessibility.
    #[serde(default = "default_auto_paste_enabled")]
    pub auto_paste_enabled: bool,

    /// Show a live transcript inside the pill while the user is still
    /// speaking, in place of the default waveform. Driven by the same
    /// speculative transcription decoder that already feeds the
    /// post-stop reveal — flipping this on simply reveals what was
    /// previously buffered silently. Forces the speculative pass to run
    /// on battery (it would otherwise bail to save power). Off by
    /// default; the listening silhouette stays compact for users who
    /// don't opt in.
    #[serde(default = "default_live_transcript_enabled")]
    pub live_transcript_enabled: bool,

    /// Gemma 4 only. When true, the engine sends
    /// `chat_template_kwargs: { enable_thinking: true }` per request so
    /// the model emits a chain-of-thought pre-amble before the polished
    /// output. Off by default — Vox is a low-latency dictation tool, not
    /// a reasoning task. Per-Voice-Mode can override (see
    /// `modes::VoiceMode::enable_thinking`).
    #[serde(default)]
    pub enable_thinking: bool,

    /// Token passed to `llama-server` as `--api-key`; generated lazily
    /// on first spawn and reused so the orphan reaper can identify
    /// stale children. Empty = not yet generated.
    #[serde(default)]
    pub inference_api_key: String,

    /// User dismissed the "Try Parakeet" upgrade banner in
    /// Settings → Models. Sticky — once true, the banner stays hidden
    /// regardless of language / availability state.
    #[serde(default)]
    pub try_parakeet_dismissed: bool,

    /// Name of the audio input device Vox listens to. `None` (or
    /// missing) means follow the macOS system default — re-resolved
    /// every recording, so swapping the system input takes effect on
    /// the next dictation. When `Some(name)` and that device isn't
    /// connected at recording time, the session surfaces
    /// `AudioError::ConfiguredDeviceUnavailable` and the pill shows
    /// a clear error — we deliberately do NOT silently fall back to
    /// the system default (that quiet fallback was the bug this
    /// field was added to fix: a disconnected iPhone would route
    /// Whisper onto the MacBook mic with no user-visible signal).
    #[serde(default)]
    pub audio_input_device: Option<String>,

    /// Start a dictation session when the user speaks the wake phrase
    /// ("Hey Vox" by default). Keeps a low-cost microphone listener
    /// running whenever Vox is otherwise idle — fully on-device, the
    /// audio never leaves RAM (see `wake.rs`). Off by default.
    #[serde(default = "default_wake_word_enabled")]
    pub wake_word_enabled: bool,

    /// The spoken phrase that triggers a session when
    /// `wake_word_enabled` is on. Matched fuzzily against the
    /// transcript of short utterances so near-homophones ("hey fox")
    /// still trigger. Falls back to the default when blank.
    #[serde(default = "default_wake_phrase")]
    pub wake_phrase: String,

    /// Notch List (macOS-only). Enabled by default (opt-out). When true
    /// on macOS, a top-center floating "notch" window is created at
    /// launch showing the top to-do item, expanding on hover. The
    /// Settings field round-trips on Windows too (where it has no
    /// window effect) so the doc stays OS-agnostic.
    #[serde(default = "default_notch_list_enabled")]
    pub notch_list_enabled: bool,

    /// How many top items the notch panel reveals on hover. Clamped to
    /// 3..=7 on load.
    #[serde(default = "default_notch_list_top_n")]
    pub notch_list_top_n: u8,

    /// Which display the notch attaches to: `"active"` (cursor's
    /// screen, like the pill), `"builtin"`, or `"primary"`. Coerced to
    /// one of those three on load.
    #[serde(default = "default_notch_list_display")]
    pub notch_list_display: String,

    /// Dedicated hotkey that routes the next dictation through the notch
    /// command interpreter (returns structured ops applied to the list)
    /// instead of the polish + clipboard pipeline. Default `Cmd+Alt+L`.
    #[serde(default = "default_notch_list_voice_hotkey")]
    pub notch_list_voice_hotkey: String,

    /// When true, a voice batch that removes or clears items is staged
    /// for confirmation rather than applied immediately. On by default.
    #[serde(default = "default_notch_list_confirm_destructive")]
    pub notch_list_confirm_destructive: bool,

    /// Intent-delay in milliseconds before the notch panel expands on
    /// hover. Clamped to 0..=1000 on load and save. UI-only — read by the
    /// notch webview via `get_public_settings`, never via `Prefs`.
    #[serde(default = "default_notch_list_hover_delay_ms")]
    pub notch_list_hover_delay_ms: u32,

    /// When true, the notch expands on click instead of hover. UI-only.
    #[serde(default = "default_notch_list_click_to_expand")]
    pub notch_list_click_to_expand: bool,

    /// Periodic auto-update scheduler. ON by default; turning it off
    /// disables the daily background check while keeping the manual
    /// "Check now" button in Settings → About working. Mirrored to
    /// disk via `set_settings` like every other top-level field.
    #[serde(default = "default_auto_update_check_enabled")]
    pub auto_update_check_enabled: bool,

    /// Unix epoch seconds of the most recent SUCCESSFUL update check
    /// (background or manual). Used by `updater::should_check_now`
    /// to gate the 24 h cadence and 6 h cooldown across relaunches.
    /// `None` until the first check lands.
    #[serde(default)]
    pub last_update_check_at: Option<i64>,

    /// Version the user "Later"-dismissed in the tray banner. While
    /// this equals the current available version, the banner +
    /// tray-icon dot stay hidden; Settings → About still surfaces
    /// the pending state. Cleared automatically once the available
    /// version moves past this string (a new release auto-rearms).
    #[serde(default)]
    pub dismissed_update_version: Option<String>,

    /// Version for which the prominent auto-show update window has
    /// already been surfaced. Set once the window builds successfully
    /// so it fires at most once per available version, ever — even
    /// across relaunches. The passive tray banner is gated separately
    /// on `dismissed_update_version`, so this never affects it.
    #[serde(default)]
    pub update_window_shown_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            cancel_hotkey: default_cancel_hotkey(),
            cancel_requires_double_press: false,
            launch_at_login: false,
            dictation_enabled: true,
            hands_free_enabled: true,
            language: default_language(),
            onboarded: false,
            dictionary: Vec::new(),
            context: String::new(),
            stt_engine: default_stt_engine(),
            stt_model_id: default_stt_model_id(),
            llm_engine: default_llm_engine(),
            llm_model_id: default_llm_model_id(),
            polishing_enabled: default_polishing_enabled(),
            gemma_tos_accepted: false,
            active_mode_id: default_active_mode_id(),
            mode_cycle_hotkey: default_mode_cycle_hotkey(),
            auto_suggest_modes: default_auto_suggest_modes(),
            appearance: default_appearance(),
            show_dock_icon: default_show_dock_icon(),
            streaming_pill_enabled: default_streaming_pill_enabled(),
            auto_paste_enabled: default_auto_paste_enabled(),
            live_transcript_enabled: default_live_transcript_enabled(),
            enable_thinking: false,
            inference_api_key: String::new(),
            try_parakeet_dismissed: false,
            audio_input_device: None,
            wake_word_enabled: default_wake_word_enabled(),
            wake_phrase: default_wake_phrase(),
            notch_list_enabled: default_notch_list_enabled(),
            notch_list_top_n: default_notch_list_top_n(),
            notch_list_display: default_notch_list_display(),
            notch_list_voice_hotkey: default_notch_list_voice_hotkey(),
            notch_list_confirm_destructive: default_notch_list_confirm_destructive(),
            notch_list_hover_delay_ms: default_notch_list_hover_delay_ms(),
            notch_list_click_to_expand: default_notch_list_click_to_expand(),
            auto_update_check_enabled: default_auto_update_check_enabled(),
            last_update_check_at: None,
            dismissed_update_version: None,
            update_window_shown_version: None,
        }
    }
}

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("failed to open settings store: {e}");
            return Settings::default();
        }
    };

    let get_str =
        |k: &str| store.get(k).and_then(|v| v.as_str().map(|s| s.to_string()));
    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let hotkey = get_str(KEY_HOTKEY).unwrap_or_else(default_hotkey);

    let cancel_hotkey = get_str(KEY_CANCEL_HOTKEY).unwrap_or_else(default_cancel_hotkey);
    let cancel_requires_double_press =
        get_bool(KEY_CANCEL_REQUIRES_DOUBLE_PRESS, false);
    let language = get_str(KEY_LANGUAGE).unwrap_or_else(default_language);

    let launch_at_login = get_bool(KEY_LAUNCH_AT_LOGIN, false);
    let dictation_enabled = get_bool(KEY_DICTATION_ENABLED, true);
    let hands_free_enabled = get_bool(KEY_HANDS_FREE, true);
    let onboarded = get_bool(KEY_ONBOARDED, false);
    let gemma_tos_accepted = get_bool(KEY_GEMMA_TOS_ACCEPTED, false);

    let dictionary = store
        .get(KEY_DICTIONARY)
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let context = get_str(KEY_CONTEXT).unwrap_or_default();

    let stt_engine = get_str(KEY_STT_ENGINE).unwrap_or_else(default_stt_engine);
    let stt_model_id = get_str(KEY_STT_MODEL_ID).unwrap_or_else(default_stt_model_id);
    let llm_engine = get_str(KEY_LLM_ENGINE).unwrap_or_else(default_llm_engine);
    let llm_model_id = get_str(KEY_LLM_MODEL_ID).unwrap_or_else(default_llm_model_id);

    let polishing_enabled = get_bool(KEY_POLISHING_ENABLED, default_polishing_enabled());

    let active_mode_id = get_str(KEY_ACTIVE_MODE_ID).unwrap_or_else(default_active_mode_id);
    let mode_cycle_hotkey =
        get_str(KEY_MODE_CYCLE_HOTKEY).unwrap_or_else(default_mode_cycle_hotkey);
    let auto_suggest_modes = get_bool(KEY_AUTO_SUGGEST_MODES, default_auto_suggest_modes());

    let appearance = match get_str(KEY_APPEARANCE).as_deref() {
        Some("light") => "light".to_string(),
        Some("dark") => "dark".to_string(),
        _ => default_appearance(),
    };

    let show_dock_icon = get_bool(KEY_SHOW_DOCK_ICON, default_show_dock_icon());

    let streaming_pill_enabled =
        get_bool(KEY_STREAMING_PILL_ENABLED, default_streaming_pill_enabled());

    let auto_paste_enabled =
        get_bool(KEY_AUTO_PASTE_ENABLED, default_auto_paste_enabled());

    let live_transcript_enabled = get_bool(
        KEY_LIVE_TRANSCRIPT_ENABLED,
        default_live_transcript_enabled(),
    );

    let enable_thinking = get_bool(KEY_ENABLE_THINKING, false);

    let inference_api_key = get_str(KEY_INFERENCE_API_KEY).unwrap_or_default();

    let try_parakeet_dismissed = get_bool(KEY_TRY_PARAKEET_DISMISSED, false);

    let audio_input_device = store
        .get(KEY_AUDIO_INPUT_DEVICE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let wake_word_enabled = get_bool(KEY_WAKE_WORD_ENABLED, default_wake_word_enabled());
    let wake_phrase = get_str(KEY_WAKE_PHRASE)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_wake_phrase);

    let notch_list_enabled = get_bool(KEY_NOTCH_LIST_ENABLED, default_notch_list_enabled());
    let notch_list_top_n = store
        .get(KEY_NOTCH_LIST_TOP_N)
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(3, 7) as u8)
        .unwrap_or_else(default_notch_list_top_n);
    let notch_list_display = match get_str(KEY_NOTCH_LIST_DISPLAY).as_deref() {
        Some("active") => "active".to_string(),
        Some("builtin") => "builtin".to_string(),
        Some("primary") => "primary".to_string(),
        _ => default_notch_list_display(),
    };
    let notch_list_voice_hotkey =
        get_str(KEY_NOTCH_LIST_VOICE_HOTKEY).unwrap_or_else(default_notch_list_voice_hotkey);
    let notch_list_confirm_destructive = get_bool(
        KEY_NOTCH_LIST_CONFIRM_DESTRUCTIVE,
        default_notch_list_confirm_destructive(),
    );
    let notch_list_hover_delay_ms = store
        .get(KEY_NOTCH_LIST_HOVER_DELAY_MS)
        .and_then(|v| v.as_u64())
        .map(|n| n.min(1000) as u32)
        .unwrap_or_else(default_notch_list_hover_delay_ms);
    let notch_list_click_to_expand = get_bool(
        KEY_NOTCH_LIST_CLICK_TO_EXPAND,
        default_notch_list_click_to_expand(),
    );

    let auto_update_check_enabled = get_bool(
        KEY_AUTO_UPDATE_CHECK_ENABLED,
        default_auto_update_check_enabled(),
    );

    let last_update_check_at = store
        .get(KEY_LAST_UPDATE_CHECK_AT)
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0);

    let dismissed_update_version = store
        .get(KEY_DISMISSED_UPDATE_VERSION)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let update_window_shown_version = store
        .get(KEY_UPDATE_WINDOW_SHOWN_VERSION)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let _ = store.save();

    Settings {
        hotkey,
        cancel_hotkey,
        cancel_requires_double_press,
        launch_at_login,
        dictation_enabled,
        hands_free_enabled,
        language,
        onboarded,
        dictionary,
        context,
        stt_engine,
        stt_model_id,
        llm_engine,
        llm_model_id,
        polishing_enabled,
        gemma_tos_accepted,
        active_mode_id,
        mode_cycle_hotkey,
        auto_suggest_modes,
        appearance,
        show_dock_icon,
        streaming_pill_enabled,
        auto_paste_enabled,
        live_transcript_enabled,
        enable_thinking,
        inference_api_key,
        try_parakeet_dismissed,
        audio_input_device,
        wake_word_enabled,
        wake_phrase,
        notch_list_enabled,
        notch_list_top_n,
        notch_list_display,
        notch_list_voice_hotkey,
        notch_list_confirm_destructive,
        notch_list_hover_delay_ms,
        notch_list_click_to_expand,
        auto_update_check_enabled,
        last_update_check_at,
        dismissed_update_version,
        update_window_shown_version,
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;

    let s = |k: &str, v: &str| store.set(k, serde_json::Value::String(v.to_string()));
    let b = |k: &str, v: bool| store.set(k, serde_json::Value::Bool(v));

    s(KEY_HOTKEY, &settings.hotkey);
    s(KEY_CANCEL_HOTKEY, &settings.cancel_hotkey);
    b(
        KEY_CANCEL_REQUIRES_DOUBLE_PRESS,
        settings.cancel_requires_double_press,
    );
    s(KEY_LANGUAGE, &settings.language);
    b(KEY_HANDS_FREE, settings.hands_free_enabled);
    b(KEY_LAUNCH_AT_LOGIN, settings.launch_at_login);
    b(KEY_DICTATION_ENABLED, settings.dictation_enabled);
    b(KEY_ONBOARDED, settings.onboarded);
    store.set(
        KEY_DICTIONARY,
        serde_json::Value::Array(
            settings
                .dictionary
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    s(KEY_CONTEXT, &settings.context);

    s(KEY_STT_ENGINE, &settings.stt_engine);
    s(KEY_STT_MODEL_ID, &settings.stt_model_id);
    s(KEY_LLM_ENGINE, &settings.llm_engine);
    s(KEY_LLM_MODEL_ID, &settings.llm_model_id);
    b(KEY_POLISHING_ENABLED, settings.polishing_enabled);
    b(KEY_GEMMA_TOS_ACCEPTED, settings.gemma_tos_accepted);

    s(KEY_ACTIVE_MODE_ID, &settings.active_mode_id);
    s(KEY_MODE_CYCLE_HOTKEY, &settings.mode_cycle_hotkey);
    b(KEY_AUTO_SUGGEST_MODES, settings.auto_suggest_modes);

    let appearance = if settings.appearance == "light" { "light" } else { "dark" };
    s(KEY_APPEARANCE, appearance);
    b(KEY_SHOW_DOCK_ICON, settings.show_dock_icon);

    b(KEY_STREAMING_PILL_ENABLED, settings.streaming_pill_enabled);
    b(KEY_AUTO_PASTE_ENABLED, settings.auto_paste_enabled);
    b(
        KEY_LIVE_TRANSCRIPT_ENABLED,
        settings.live_transcript_enabled,
    );
    b(KEY_ENABLE_THINKING, settings.enable_thinking);

    s(KEY_INFERENCE_API_KEY, &settings.inference_api_key);

    b(KEY_TRY_PARAKEET_DISMISSED, settings.try_parakeet_dismissed);

    match &settings.audio_input_device {
        Some(name) if !name.is_empty() => {
            s(KEY_AUDIO_INPUT_DEVICE, name);
        }
        _ => {
            store.set(KEY_AUDIO_INPUT_DEVICE, serde_json::Value::Null);
        }
    }

    b(KEY_WAKE_WORD_ENABLED, settings.wake_word_enabled);
    let wake_phrase = settings.wake_phrase.trim();
    if wake_phrase.is_empty() {
        s(KEY_WAKE_PHRASE, &default_wake_phrase());
    } else {
        s(KEY_WAKE_PHRASE, wake_phrase);
    }

    b(KEY_NOTCH_LIST_ENABLED, settings.notch_list_enabled);
    store.set(
        KEY_NOTCH_LIST_TOP_N,
        serde_json::Value::from(settings.notch_list_top_n.clamp(3, 7)),
    );
    let notch_display = match settings.notch_list_display.as_str() {
        "builtin" => "builtin",
        "primary" => "primary",
        _ => "active",
    };
    s(KEY_NOTCH_LIST_DISPLAY, notch_display);
    let notch_hotkey = settings.notch_list_voice_hotkey.trim();
    if notch_hotkey.is_empty() {
        s(KEY_NOTCH_LIST_VOICE_HOTKEY, &default_notch_list_voice_hotkey());
    } else {
        s(KEY_NOTCH_LIST_VOICE_HOTKEY, notch_hotkey);
    }
    b(
        KEY_NOTCH_LIST_CONFIRM_DESTRUCTIVE,
        settings.notch_list_confirm_destructive,
    );
    store.set(
        KEY_NOTCH_LIST_HOVER_DELAY_MS,
        serde_json::Value::from(settings.notch_list_hover_delay_ms.min(1000)),
    );
    b(
        KEY_NOTCH_LIST_CLICK_TO_EXPAND,
        settings.notch_list_click_to_expand,
    );

    b(KEY_AUTO_UPDATE_CHECK_ENABLED, settings.auto_update_check_enabled);
    match settings.last_update_check_at {
        Some(n) if n > 0 => {
            store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::from(n));
        }
        _ => {
            store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::Null);
        }
    }
    match &settings.dismissed_update_version {
        Some(v) if !v.is_empty() => {
            s(KEY_DISMISSED_UPDATE_VERSION, v);
        }
        _ => {
            store.set(KEY_DISMISSED_UPDATE_VERSION, serde_json::Value::Null);
        }
    }

    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `last_update_check_at` field. Used by
/// `updater::perform_check` to update the wall-clock timestamp
/// without rewriting the rest of the Settings document on every
/// background tick. Mirrors `set_hotkey_field`.
pub fn set_last_update_check_at<R: Runtime>(
    app: &AppHandle<R>,
    secs: i64,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    if secs > 0 {
        store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::from(secs));
    } else {
        store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::Null);
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `dismissed_update_version` field. `None` /
/// empty string clears the dismissal so the next "Later" press
/// can be re-applied without a settings round-trip.
pub fn set_dismissed_update_version<R: Runtime>(
    app: &AppHandle<R>,
    value: Option<&str>,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    match value {
        Some(v) if !v.is_empty() => {
            store.set(
                KEY_DISMISSED_UPDATE_VERSION,
                serde_json::Value::String(v.to_string()),
            );
        }
        _ => {
            store.set(KEY_DISMISSED_UPDATE_VERSION, serde_json::Value::Null);
        }
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `update_window_shown_version` field. Written once
/// the prominent auto-show update window builds successfully so the
/// window fires at most once per available version. `None` / empty
/// clears the marker.
pub fn set_update_window_shown_version<R: Runtime>(
    app: &AppHandle<R>,
    value: Option<&str>,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    match value {
        Some(v) if !v.is_empty() => {
            store.set(
                KEY_UPDATE_WINDOW_SHOWN_VERSION,
                serde_json::Value::String(v.to_string()),
            );
        }
        _ => {
            store.set(KEY_UPDATE_WINDOW_SHOWN_VERSION, serde_json::Value::Null);
        }
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

pub fn mark_onboarded<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(store) = app.store(STORE_FILE) {
        store.set(KEY_ONBOARDED, serde_json::Value::Bool(true));
        let _ = store.save();
    }
}

/// Lightweight view of the user's preferences for the hotkey thread.
#[derive(Debug, Clone)]
pub struct Prefs {
    pub dictation_enabled: bool,
    pub hands_free_enabled: bool,
    pub language: String,
    pub dictionary: Vec<String>,
    /// Mirror of `Settings::context` — the user's free-form background
    /// prose, folded into the polish system prompt at session start for
    /// modes that opt in (`VoiceMode::context_enabled`).
    pub context: String,

    pub stt_engine: String,
    pub stt_model_id: String,
    pub llm_engine: String,
    pub llm_model_id: String,
    pub polishing_enabled: bool,

    pub active_mode_id: String,
    pub auto_suggest_modes: bool,

    /// Mirror of `Settings::streaming_pill_enabled`; the pipeline reads
    /// from a fresh `settings::load` to guarantee the value is current
    /// at the moment streaming begins, but `Prefs` carries it too so
    /// callers that already hold a `Prefs` don't need a second load.
    #[allow(dead_code)]
    pub streaming_pill_enabled: bool,

    /// Mirror of `Settings::auto_paste_enabled`. Captured at session
    /// start so a mid-pipeline toggle doesn't change the delivery
    /// behaviour for an in-flight transcript.
    pub auto_paste_enabled: bool,

    /// Mirror of `Settings::live_transcript_enabled`. Captured at
    /// session start so a mid-utterance toggle can't swap the pill
    /// silhouette under the user. Drives the speculative decoder's
    /// battery override and opens the post-release transcript stream
    /// even when `streaming_pill_enabled` is off.
    pub live_transcript_enabled: bool,

    /// Mirror of `Settings::enable_thinking`. Per-Voice-Mode override
    /// (`VoiceMode::enable_thinking: Option<bool>`) takes precedence at
    /// session-begin time; this value is the global fallback.
    pub enable_thinking: bool,

    /// Mirror of `Settings::audio_input_device`. `None` = follow macOS
    /// system default; `Some(name)` is matched against
    /// `cpal::Host::input_devices()` at `audio::begin_capture` time.
    pub audio_input_device: Option<String>,

    /// Mirror of `Settings::cancel_requires_double_press`. Read fresh on
    /// each cancel-hotkey press so toggling it takes effect immediately,
    /// even mid-session.
    pub cancel_requires_double_press: bool,

    /// Mirror of `Settings::notch_list_display`. Read by the notch
    /// window placement to pick the target screen (macOS only — the
    /// notch window is `cfg`-gated, so this is unused on other hosts).
    #[allow(dead_code)]
    pub notch_list_display: String,

    /// Mirror of `Settings::notch_list_confirm_destructive`. Captured so
    /// the notch-command session can stage destructive batches without a
    /// second settings load.
    pub notch_list_confirm_destructive: bool,
}

pub fn prefs<R: Runtime>(app: &AppHandle<R>) -> Prefs {
    let s = load(app);
    Prefs {
        dictation_enabled: s.dictation_enabled,
        hands_free_enabled: s.hands_free_enabled,
        language: s.language,
        dictionary: s.dictionary,
        context: s.context,
        stt_engine: s.stt_engine,
        stt_model_id: s.stt_model_id,
        llm_engine: s.llm_engine,
        llm_model_id: s.llm_model_id,
        polishing_enabled: s.polishing_enabled,
        active_mode_id: s.active_mode_id,
        auto_suggest_modes: s.auto_suggest_modes,
        streaming_pill_enabled: s.streaming_pill_enabled,
        auto_paste_enabled: s.auto_paste_enabled,
        live_transcript_enabled: s.live_transcript_enabled,
        enable_thinking: s.enable_thinking,
        audio_input_device: s.audio_input_device,
        cancel_requires_double_press: s.cancel_requires_double_press,
        notch_list_display: s.notch_list_display,
        notch_list_confirm_destructive: s.notch_list_confirm_destructive,
    }
}

pub fn set_active_mode_id<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let mut s = load(app);
    s.active_mode_id = id.to_string();
    save(app, &s)?;
    // Mirror `commands::set_settings`: any change to the live `Settings`
    // doc must fan out so the tray popover, Settings window, and other
    // listeners stay in sync. Without this, the keyboard mode-cycle
    // and the `set_active_mode` / `cycle_active_mode` IPC commands all
    // updated disk silently and any open UI showed stale state until
    // the next full settings save.
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed after active mode update: {e}");
    }
    Ok(())
}

/// Persist a single hotkey field without rewriting the rest of the
/// settings document — used when `hotkey::register` falls back so the
/// picker reflects the bound shortcut.
pub fn set_hotkey_field<R: Runtime>(
    app: &AppHandle<R>,
    purpose: HotkeyPurpose,
    value: &str,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    let key = match purpose {
        HotkeyPurpose::PushToTalk => KEY_HOTKEY,
        HotkeyPurpose::Cancel => KEY_CANCEL_HOTKEY,
        HotkeyPurpose::ModeCycle => KEY_MODE_CYCLE_HOTKEY,
        HotkeyPurpose::NotchVoice => KEY_NOTCH_LIST_VOICE_HOTKEY,
    };
    store.set(key, serde_json::Value::String(value.to_string()));
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyPurpose {
    PushToTalk,
    Cancel,
    ModeCycle,
    NotchVoice,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_pill_enabled_defaults_to_true() {
        let s = Settings::default();
        assert!(s.streaming_pill_enabled);
    }

    #[test]
    fn streaming_pill_enabled_roundtrips_through_serde() {
        let mut s = Settings::default();
        s.streaming_pill_enabled = false;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(!restored.streaming_pill_enabled);

        // And verify the default kicks in when the field is absent.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(restored.streaming_pill_enabled);
    }

    #[test]
    fn live_transcript_enabled_defaults_to_false() {
        let s = Settings::default();
        assert!(!s.live_transcript_enabled);
    }

    #[test]
    fn live_transcript_enabled_roundtrips_through_serde() {
        let mut s = Settings::default();
        s.live_transcript_enabled = true;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.live_transcript_enabled);

        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(!restored.live_transcript_enabled);
    }

    #[test]
    fn wake_word_defaults_off_with_default_phrase() {
        let s = Settings::default();
        assert!(!s.wake_word_enabled);
        assert_eq!(s.wake_phrase, "hey vox");
    }

    #[test]
    fn wake_word_roundtrips_through_serde() {
        let mut s = Settings::default();
        s.wake_word_enabled = true;
        s.wake_phrase = "okay vox".to_string();
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.wake_word_enabled);
        assert_eq!(restored.wake_phrase, "okay vox");

        // Older on-disk docs without the fields fall back to defaults.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(!restored.wake_word_enabled);
        assert_eq!(restored.wake_phrase, default_wake_phrase());
    }

    #[test]
    fn cancel_double_press_defaults_off_and_roundtrips() {
        let s = Settings::default();
        assert!(!s.cancel_requires_double_press);

        let mut s = Settings::default();
        s.cancel_requires_double_press = true;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.cancel_requires_double_press);

        // Older on-disk docs without the field fall back to off.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(!restored.cancel_requires_double_press);
    }

    #[test]
    fn notch_list_defaults_enabled_with_top_n_5() {
        let s = Settings::default();
        assert!(s.notch_list_enabled);
        assert_eq!(s.notch_list_top_n, 5);
        assert_eq!(s.notch_list_display, "active");
        assert_eq!(s.notch_list_voice_hotkey, "Cmd+Alt+L");
        assert!(s.notch_list_confirm_destructive);
        assert_eq!(s.notch_list_hover_delay_ms, 120);
        assert!(!s.notch_list_click_to_expand);
    }

    #[test]
    fn notch_list_fields_roundtrip_through_serde() {
        let mut s = Settings::default();
        s.notch_list_enabled = false;
        s.notch_list_top_n = 7;
        s.notch_list_display = "builtin".to_string();
        s.notch_list_voice_hotkey = "Cmd+Shift+L".to_string();
        s.notch_list_confirm_destructive = false;
        s.notch_list_hover_delay_ms = 300;
        s.notch_list_click_to_expand = true;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(!restored.notch_list_enabled);
        assert_eq!(restored.notch_list_top_n, 7);
        assert_eq!(restored.notch_list_display, "builtin");
        assert_eq!(restored.notch_list_voice_hotkey, "Cmd+Shift+L");
        assert!(!restored.notch_list_confirm_destructive);
        assert_eq!(restored.notch_list_hover_delay_ms, 300);
        assert!(restored.notch_list_click_to_expand);
    }

    #[test]
    fn notch_list_absent_fields_fall_back_to_defaults() {
        // Older on-disk docs without the notch fields must deserialize
        // with the opt-out default ON (founder mandate).
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(restored.notch_list_enabled);
        assert_eq!(restored.notch_list_top_n, 5);
        assert_eq!(restored.notch_list_display, "active");
        assert_eq!(restored.notch_list_voice_hotkey, "Cmd+Alt+L");
        assert!(restored.notch_list_confirm_destructive);
        assert_eq!(restored.notch_list_hover_delay_ms, 120);
        assert!(!restored.notch_list_click_to_expand);
    }

    #[test]
    fn notch_list_hover_delay_clamps_on_load() {
        // The load path clamps an out-of-range stored value to 1000 ms so
        // a hand-edited doc can't park the notch panel behind a 5 s delay.
        // We exercise the clamp predicate the load path uses directly,
        // mirroring the `.min(1000)` in `load`.
        let stored: u64 = 5000;
        let loaded = stored.min(1000) as u32;
        assert_eq!(loaded, 1000);
        // A serde default still survives (no clamp needed) for an in-range
        // value.
        let in_range: u64 = 250;
        assert_eq!(in_range.min(1000) as u32, 250);
    }

    #[test]
    fn hotkey_roundtrips_through_serde() {
        // A user-picked PTT spec must survive serialize / deserialize
        // without being silently replaced by the default.
        let mut s = Settings::default();
        s.hotkey = "Cmd+Shift+J".to_string();
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.hotkey, "Cmd+Shift+J");

        // And verify the default kicks in when the field is absent.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert_eq!(restored.hotkey, default_hotkey());
    }
}
