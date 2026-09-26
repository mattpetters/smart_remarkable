//! Small device-owned preferences API for the native settings overlay.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use warp::{Filter, Reply};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Preferences {
    pub backend: Backend,
    pub reply_length: ReplyLength,
    pub ink_color: InkColor,
    pub page_context: bool,
    pub auto_fallback: bool,
    pub backend_order: Vec<Backend>,
    pub models: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Codex,
    Hermes,
    Claude,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReplyLength {
    Brief,
    Balanced,
    Detailed,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InkColor {
    #[default]
    Blue,
    Red,
    Cyan,
    Magenta,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            backend: Backend::Codex,
            reply_length: ReplyLength::Balanced,
            ink_color: InkColor::Blue,
            page_context: true,
            auto_fallback: true,
            backend_order: vec![Backend::Codex, Backend::Hermes, Backend::Claude],
            models: Default::default(),
        }
    }
}
impl Backend {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Hermes => "hermes",
            Self::Claude => "claude",
        }
    }
}
impl Preferences {
    /// Device rendering preferences stay local and must not extend the bridge
    /// protocol: older bridges reject unknown request fields.
    pub fn inference_settings(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": self.backend,
            "reply_length": self.reply_length,
            "page_context": self.page_context,
            "auto_fallback": self.auto_fallback,
            "backend_order": self.backend_order,
            "models": self.models,
        })
    }
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.backend_order.is_empty() && self.backend_order.len() <= 3, "Invalid fallback order");
        for (i, backend) in self.backend_order.iter().enumerate() {
            anyhow::ensure!(!self.backend_order[..i].contains(backend), "Duplicate fallback provider");
        }
        for (provider, model) in &self.models {
            anyhow::ensure!(
                ["codex", "hermes", "claude"].contains(&provider.as_str())
                    && !model.is_empty()
                    && model.len() <= 96
                    && model.chars().all(|c| c.is_ascii_alphanumeric() || "._/+:-".contains(c))
                    && model.chars().next().unwrap().is_ascii_alphanumeric(),
                "Invalid model"
            );
        }
        Ok(())
    }
}
fn path() -> PathBuf {
    std::env::var_os("REMARKABLE_PREFERENCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/root/smart-remarkable/preferences.json"))
}
pub fn load() -> Result<Preferences> {
    match std::fs::read(path()) {
        Ok(bytes) => {
            let prefs: Preferences = serde_json::from_slice(&bytes)?;
            prefs.validate()?;
            Ok(prefs)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}
fn save_to(path: &std::path::Path, prefs: &Preferences) -> Result<()> {
    prefs.validate()?;
    let temporary = path.with_extension("json.new");
    std::fs::write(&temporary, serde_json::to_vec_pretty(prefs)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
#[derive(Default)]
struct State {
    opened: Option<Instant>,
    busy: bool,
    send: bool,
    dispatched: bool,
    label: String,
    phase: String,
    catalog: Option<serde_json::Value>,
}
static STATE: Mutex<State> = Mutex::new(State {
    opened: None,
    busy: false,
    send: false,
    dispatched: false,
    label: String::new(),
    phase: String::new(),
    catalog: None,
});
impl State {
    fn open(&self) -> bool {
        self.opened.is_some_and(|t| t.elapsed() < Duration::from_secs(300))
    }
    fn request_send(&mut self) -> bool {
        if self.open() || self.busy || self.send {
            return false;
        }
        self.send = true;
        true
    }
}
pub fn answer_label() -> String {
    STATE.lock().map(|s| s.label.clone()).unwrap_or_default()
}
pub fn set_provider(provider: &str, model: &str) {
    // Keep provider metadata bounded and safe for SVG / device status output.
    if ["codex", "claude", "hermes"].contains(&provider) && model.len() <= 96 && model.chars().all(|c| c.is_ascii_alphanumeric() || "._/+:-".contains(c)) {
        if let Ok(mut s) = STATE.lock() {
            s.label = format!("{provider} / {model}");
            s.phase = "Thinking".into();
        }
    }
}
pub fn set_phase(phase: &str) {
    if let Ok(mut s) = STATE.lock() {
        s.phase = phase.to_string();
    }
}
pub async fn refresh_catalog() {
    let Ok(token) = std::env::var("OPENAI_API_KEY") else {
        return;
    };
    let result = reqwest::Client::new()
        .get("http://127.0.0.1:8765/backends")
        .bearer_auth(token)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    if let Ok(response) = result {
        if response.status().is_success() {
            if let Ok(value) = response.json::<serde_json::Value>().await {
                if let Ok(mut s) = STATE.lock() {
                    s.catalog = Some(value);
                }
            }
        }
    }
}
pub async fn prepare_label() {
    refresh_catalog().await;
    if let Ok(prefs) = load() {
        let provider = prefs.backend.name();
        let model = prefs
            .models
            .get(provider)
            .cloned()
            .or_else(|| {
                STATE
                    .lock()
                    .ok()
                    .and_then(|s| s.catalog.as_ref()?.get(provider)?.get("model")?.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| "default".into());
        set_provider(provider, &model);
        set_phase("Connecting");
    }
}
pub fn is_open() -> bool {
    STATE.lock().map(|s| s.open()).unwrap_or(false)
}
pub fn toggle() {
    if let Ok(mut s) = STATE.lock() {
        if !s.busy && !s.send {
            s.opened = if s.open() { None } else { Some(Instant::now()) };
        }
    }
}
fn close() {
    if let Ok(mut s) = STATE.lock() {
        s.opened = None;
    }
}
pub fn take_send() -> bool {
    STATE
        .lock()
        .map(|mut s| {
            if s.send && !s.dispatched {
                s.dispatched = true;
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
}
pub fn begin_answer() -> bool {
    STATE
        .lock()
        .map(|mut s| {
            if s.open() || s.busy {
                false
            } else {
                s.busy = true;
                s.phase = "Connecting".into();
                s.label.clear();
                true
            }
        })
        .unwrap_or(false)
}
pub fn finish_answer() {
    if let Ok(mut s) = STATE.lock() {
        s.busy = false;
        s.send = false;
        s.dispatched = false;
        s.phase.clear();
    }
}

/// Loopback only. No credentials, shell commands, URLs or arbitrary paths can
/// be configured here. Mutations require an open panel and a custom header.
pub fn start() {
    tokio::spawn(async {
        loop {
            refresh_catalog().await;
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
    let get = warp::path("settings").and(warp::path::end()).and(warp::get()).map(|| match load() {
        Ok(prefs) => {
            let state = STATE.lock().unwrap();
            warp::reply::json(&serde_json::json!({"open": state.open(), "busy": state.busy || state.send, "preferences": prefs, "provider_label": state.label, "phase": state.phase, "catalog": state.catalog})).into_response()
        }
        Err(_) => warp::reply::with_status("Preferences unavailable", warp::http::StatusCode::INTERNAL_SERVER_ERROR).into_response(),
    });
    let put = warp::path("settings")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::header::exact("x-smart-remarkable", "1"))
        .and(warp::body::content_length_limit(4096))
        .and(warp::body::json())
        .map(|prefs: Preferences| {
            if !is_open() {
                return warp::reply::with_status("Panel is closed", warp::http::StatusCode::CONFLICT).into_response();
            }
            match save_to(&path(), &prefs) {
                Ok(()) => {
                    close();
                    warp::reply::json(&prefs).into_response()
                }
                Err(_) => warp::reply::with_status("Could not save settings", warp::http::StatusCode::INTERNAL_SERVER_ERROR).into_response(),
            }
        });
    let cancel = warp::path("settings")
        .and(warp::path("close"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::header::exact("x-smart-remarkable", "1"))
        .map(|| {
            close();
            warp::reply()
        });
    let open = warp::path!("settings" / "open")
        .and(warp::post())
        .and(warp::header::exact("x-smart-remarkable", "1"))
        .map(|| {
            let mut state = STATE.lock().unwrap();
            let ok = !state.busy && !state.send;
            if ok {
                state.opened = Some(Instant::now());
            }
            warp::reply::with_status("", if ok { warp::http::StatusCode::OK } else { warp::http::StatusCode::CONFLICT })
        });
    let send = warp::path!("settings" / "send")
        .and(warp::post())
        .and(warp::header::exact("x-smart-remarkable", "1"))
        .map(|| {
            let ok = STATE.lock().unwrap().request_send();
            warp::reply::with_status(
                "",
                if ok {
                    warp::http::StatusCode::ACCEPTED
                } else {
                    warp::http::StatusCode::CONFLICT
                },
            )
        });
    tokio::spawn(async move {
        match warp::serve(get.or(put).or(cancel).or(open).or(send)).try_bind_ephemeral(([127, 0, 0, 1], 8766)) {
            Ok((_, server)) => server.await,
            Err(_) => log::error!("Device settings API could not bind loopback port"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_toolbar_sends_and_sends_during_settings_are_rejected() {
        let mut state = State::default();
        assert!(state.request_send());
        assert!(!state.request_send());
        state.send = false;
        state.busy = true;
        assert!(!state.request_send());
        state.busy = false;
        state.opened = Some(Instant::now());
        assert!(!state.request_send());
        state.opened = None;
        assert!(state.request_send());
    }
    #[test]
    fn validates_provider_priority_and_model_names() {
        let mut prefs = Preferences::default();
        prefs.backend_order = vec![Backend::Claude, Backend::Hermes, Backend::Codex];
        prefs.models.insert("claude".into(), "sonnet".into());
        assert!(prefs.validate().is_ok());
        prefs.backend_order.push(Backend::Claude);
        assert!(prefs.validate().is_err());
        prefs.backend_order = vec![Backend::Claude];
        prefs.models.insert("claude".into(), "--unsafe".into());
        assert!(prefs.validate().is_err());
    }
    #[test]
    fn validates_settings_and_round_trips_atomic_file() {
        for raw in [r#"{"backend":"cloud"}"#, r#"{"page_context":"false"}"#, r#"{"api_key":"secret"}"#, r#"{"ink_color":"green"}"#] {
            assert!(serde_json::from_str::<Preferences>(raw).is_err());
        }
        let prefs: Preferences = serde_json::from_str(r#"{"backend":"hermes","reply_length":"brief","page_context":false}"#).unwrap();
        assert_eq!(prefs.ink_color, InkColor::Blue);
        assert!(prefs.inference_settings().get("ink_color").is_none());
        let path = std::env::temp_dir().join(format!("remarkable-preferences-{}.json", std::process::id()));
        save_to(&path, &prefs).unwrap();
        assert_eq!(serde_json::from_slice::<Preferences>(&std::fs::read(&path).unwrap()).unwrap(), prefs);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn ink_choices_persist_without_changing_inference_settings() {
        let original = Preferences::default();
        for color in [InkColor::Blue, InkColor::Red, InkColor::Cyan, InkColor::Magenta] {
            let prefs = Preferences { ink_color: color, ..original.clone() };
            let encoded = serde_json::to_vec(&prefs).unwrap();
            assert_eq!(serde_json::from_slice::<Preferences>(&encoded).unwrap(), prefs);
            assert_eq!(prefs.inference_settings(), original.inference_settings());
        }
    }
}
