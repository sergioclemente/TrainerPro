//! Private browser lifetime and one-shot sign-in completion. No remote-page IPC.

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::{oneshot, watch, Mutex};

use super::{api_error, persist_connection, AppError, AppState, GarminClient, PROVIDER_ID, R};
use crate::commands::providers::ProviderOperations;

const SIGN_IN_WIDTH_PX: f64 = 440.0;
const SIGN_IN_HEIGHT_PX: f64 = 560.0;

#[derive(Default)]
pub struct GarminSignInState(Mutex<Option<BrowserAttempt>>);

struct BrowserAttempt {
    id: String,
    ticket: Option<oneshot::Sender<String>>,
    cancelled: watch::Sender<bool>,
    committing: bool,
}

impl GarminSignInState {
    pub async fn cancel(&self) {
        if let Some(attempt) = self.0.lock().await.as_ref() {
            if !attempt.committing {
                attempt.cancelled.send_replace(true);
            }
        }
    }

    async fn window_closed(&self, id: &str) {
        if let Some(attempt) = self.0.lock().await.as_ref() {
            if attempt.id == id && !attempt.committing {
                attempt.cancelled.send_replace(true);
            }
        }
    }

    async fn callback(&self, id: &str, url: String) {
        if let Some(attempt) = self.0.lock().await.as_mut() {
            if attempt.id == id && !*attempt.cancelled.borrow() {
                if let Some(sender) = attempt.ticket.take() {
                    let _ = sender.send(url);
                }
            }
        }
    }

    // Cancellation wins until this boundary. Credential persistence then runs
    // to completion, so dropping an async keyring task cannot leave orphan tokens.
    async fn begin_commit(&self, id: &str) -> bool {
        let mut pending = self.0.lock().await;
        let Some(attempt) = pending.as_mut().filter(|attempt| attempt.id == id) else {
            return false;
        };
        if *attempt.cancelled.borrow() {
            return false;
        }
        attempt.committing = true;
        true
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GarminSignInResult {
    Connected,
    Cancelled,
}

fn allow_navigation(url: &tauri::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some("sso.garmin.com" | "connect.garmin.com")
        )
}

#[tauri::command]
pub async fn connect_garmin(
    app: AppHandle,
    state: State<'_, AppState>,
    operations: State<'_, GarminSignInState>,
    providers: State<'_, ProviderOperations>,
    attempt_id: String,
) -> R<GarminSignInResult> {
    // Account changes and capability operations remain serialized. Cancellation
    // only signals this attempt and deliberately does not acquire this guard.
    let _operation = providers.acquire(PROVIDER_ID)?;
    let attempt_id = uuid::Uuid::parse_str(&attempt_id)
        .map_err(|_| AppError::new("garmin_browser", "Invalid Garmin sign-in attempt."))?;
    let id = format!("garmin-sign-in-{attempt_id}");
    let (ticket_sender, ticket_receiver) = oneshot::channel();
    let (cancel_sender, mut cancelled) = watch::channel(false);
    *operations.0.lock().await = Some(BrowserAttempt {
        id: id.clone(),
        ticket: Some(ticket_sender),
        cancelled: cancel_sender,
        committing: false,
    });

    let callback_app = app.clone();
    let callback_id = id.clone();
    let window = WebviewWindowBuilder::new(
        &app,
        &id,
        WebviewUrl::External(
            GarminClient::browser_sign_in_url()
                .parse()
                .expect("static Garmin URL"),
        ),
    )
    .title("Sign in to Garmin")
    .inner_size(SIGN_IN_WIDTH_PX, SIGN_IN_HEIGHT_PX)
    .center()
    .resizable(true)
    .incognito(true)
    .devtools(false)
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_navigation(move |url| {
        if GarminClient::is_browser_sign_in_callback(url.as_str()) {
            let app = callback_app.clone();
            let id = callback_id.clone();
            let url = url.to_string();
            tauri::async_runtime::spawn(async move {
                app.state::<GarminSignInState>().callback(&id, url).await;
            });
            return false;
        }
        allow_navigation(url)
    })
    .build();
    let result = match window {
        Err(_) => Err(AppError::new(
            "garmin_browser",
            "Could not open Garmin sign-in. Try again.",
        )),
        Ok(window) => {
            let close_app = app.clone();
            let close_id = id.clone();
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed) {
                    let app = close_app.clone();
                    let id = close_id.clone();
                    tauri::async_runtime::spawn(async move {
                        app.state::<GarminSignInState>().window_closed(&id).await;
                    });
                }
            });
            let authenticated = tokio::select! {
                biased;
                _ = cancelled.changed() => Ok(None),
                result = async {
                    let callback = ticket_receiver.await.map_err(|_| AppError::new("garmin_browser", "Garmin sign-in closed before completing."))?;
                    let client = GarminClient::new().map_err(api_error)?;
                    let tokens = client.sign_in_with_browser_ticket(&callback).await.map_err(api_error)?;
                    let account = GarminClient::new().map_err(api_error)?.account(&tokens).await.map_err(api_error)?;
                    R::Ok(Some((tokens, account)))
                } => result,
            };
            match authenticated {
                Ok(Some((tokens, account))) if operations.begin_commit(&id).await => {
                    persist_connection(&state, tokens, account)
                        .await
                        .map(|_| GarminSignInResult::Connected)
                }
                Ok(_) => Ok(GarminSignInResult::Cancelled),
                Err(error) => Err(error),
            }
        }
    };
    *operations.0.lock().await = None;
    if let Some(window) = app.get_webview_window(&id) {
        let _ = window.close();
    }
    result
}

#[tauri::command]
pub async fn cancel_garmin_sign_in(
    operations: State<'_, GarminSignInState>,
    attempt_id: String,
) -> R<()> {
    let attempt_id = uuid::Uuid::parse_str(&attempt_id)
        .map_err(|_| AppError::new("garmin_browser", "Invalid Garmin sign-in attempt."))?;
    operations
        .window_closed(&format!("garmin-sign-in-{attempt_id}"))
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn attempt(state: &GarminSignInState, id: &str) -> oneshot::Receiver<String> {
        let (ticket, receiver) = oneshot::channel();
        let (cancelled, _) = watch::channel(false);
        *state.0.lock().await = Some(BrowserAttempt {
            id: id.into(),
            ticket: Some(ticket),
            cancelled,
            committing: false,
        });
        receiver
    }

    #[tokio::test]
    async fn callbacks_are_once_and_scoped_to_the_current_attempt() {
        let state = GarminSignInState::default();
        let receiver = attempt(&state, "current").await;
        state.callback("old", "wrong".into()).await;
        state.callback("current", "first".into()).await;
        state.callback("current", "second".into()).await;
        assert_eq!(receiver.await.unwrap(), "first");
    }

    #[tokio::test]
    async fn cancellation_prevents_late_callbacks_and_commit() {
        let state = GarminSignInState::default();
        let mut receiver = attempt(&state, "current").await;
        state.window_closed("old").await;
        state.cancel().await;
        state.callback("current", "late".into()).await;
        assert!(receiver.try_recv().is_err());
        assert!(!state.begin_commit("current").await);
    }

    #[tokio::test]
    async fn a_commit_cannot_be_cancelled_partway_through_persistence() {
        let state = GarminSignInState::default();
        let _receiver = attempt(&state, "current").await;
        assert!(state.begin_commit("current").await);
        state.window_closed("current").await;
        state.cancel().await;
        assert!(!*state.0.lock().await.as_ref().unwrap().cancelled.borrow());
    }

    #[test]
    fn remote_navigations_cannot_escape_garmin_https() {
        for url in [
            "http://sso.garmin.com/",
            "https://sso.garmin.com.attacker.test/",
            "tauri://localhost",
            "https://user@sso.garmin.com/",
        ] {
            assert!(!allow_navigation(&url.parse().unwrap()));
        }
        assert!(allow_navigation(
            &GarminClient::browser_sign_in_url().parse().unwrap()
        ));
    }
}
