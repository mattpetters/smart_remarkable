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
    pub page_context: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Codex,
    Hermes,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReplyLength {
    Brief,
    Balanced,
    Detailed,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            backend: Backend::Codex,
            reply_length: ReplyLength::Balanced,
            page_context: true,
        }
    }
}
fn path() -> PathBuf {
    std::env::var_os("REMARKABLE_PREFERENCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/root/smart-remarkable/preferences.json"))
}
pub fn load() -> Result<Preferences> {
    match std::fs::read(path()) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}
fn save_to(path: &std::path::Path, prefs: &Preferences) -> Result<()> {
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
}
static STATE: Mutex<State> = Mutex::new(State {
    opened: None,
    busy: false,
    send: false,
    dispatched: false,
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
    }
}

/// Loopback only. No credentials, shell commands, URLs or arbitrary paths can
/// be configured here. Mutations require an open panel and a custom header.
pub fn start() {
    let get = warp::path("settings").and(warp::path::end()).and(warp::get()).map(|| match load() {
        Ok(prefs) => {
            let state = STATE.lock().unwrap();
            warp::reply::json(&serde_json::json!({"open": state.open(), "busy": state.busy || state.send, "preferences": prefs})).into_response()
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
    fn validates_settings_and_round_trips_atomic_file() {
        for raw in [r#"{"backend":"cloud"}"#, r#"{"page_context":"false"}"#, r#"{"api_key":"secret"}"#] {
            assert!(serde_json::from_str::<Preferences>(raw).is_err());
        }
        let prefs: Preferences = serde_json::from_str(r#"{"backend":"hermes","reply_length":"brief","page_context":false}"#).unwrap();
        let path = std::env::temp_dir().join(format!("remarkable-preferences-{}.json", std::process::id()));
        save_to(&path, &prefs).unwrap();
        assert_eq!(serde_json::from_slice::<Preferences>(&std::fs::read(&path).unwrap()).unwrap(), prefs);
        std::fs::remove_file(path).unwrap();
    }
}
