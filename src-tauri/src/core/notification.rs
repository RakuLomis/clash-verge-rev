use crate::utils::window_manager::WindowManager;
use clash_verge_logging::{Type, logging};
use serde_json::json;
use smartstring::alias::String;

use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;
use tauri::{Emitter as _, WebviewWindow};

#[derive(Default)]
struct PendingProfileRefresh(Mutex<Option<Option<String>>>);

impl PendingProfileRefresh {
    fn replace(&self, profile: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(profile);
    }

    fn take(&self) -> Option<Option<String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

struct ProfileRefreshQueue {
    pending: Arc<PendingProfileRefresh>,
    wake: mpsc::SyncSender<()>,
}

impl ProfileRefreshQueue {
    fn start() -> Self {
        let pending = Arc::new(PendingProfileRefresh::default());
        let consumer = pending.clone();
        let (wake, receiver) = mpsc::sync_channel(1);
        // Exactly one dedicated thread isolates native window calls from Tokio.
        // It never holds the pending lock while accessing the window manager.
        let started = std::thread::Builder::new().name("profile-notifications".into()).spawn(move || {
            while receiver.recv().is_ok() {
                let Some(profile) = consumer.take() else { continue };
                let (done, completed) = mpsc::sync_channel(1);
                let scheduled = crate::core::handle::Handle::app_handle().run_on_main_thread(move || {
                    NotificationSystem::send_event(FrontendEvent::RefreshClash);
                    if let Some(profile) = profile {
                        NotificationSystem::send_event(FrontendEvent::ProfileChanged { current_profile_id: &profile });
                    }
                    let _ = done.try_send(());
                });
                if let Err(error) = scheduled {
                    logging!(warn, Type::Frontend, "Profile refresh scheduling failed: {error}");
                    continue;
                }
                if let Err(mpsc::RecvTimeoutError::Timeout) = completed.recv_timeout(Duration::from_secs(5)) {
                    logging!(warn, Type::Frontend, "PROFILE_UI_REFRESH_STALLED: main thread did not acknowledge refresh; pending refreshes are coalesced");
                    // A timeout cannot cancel a native call. Do not spawn replacements.
                    let _ = completed.recv();
                }
            }
        });
        if let Err(error) = started {
            logging!(warn, Type::Frontend, "Profile refresh worker unavailable: {error}");
        }
        Self { pending, wake }
    }

    fn enqueue(&self, profile: Option<String>) {
        self.pending.replace(profile);
        let _ = self.wake.try_send(());
    }
}

#[derive(Debug)]
pub enum FrontendEvent<'a> {
    RefreshClash,
    RefreshVerge,
    NoticeMessage { status: &'a str, message: String },
    ProfileChanged { current_profile_id: &'a String },
    TimerUpdated { profile_index: &'a String },
    ProfileUpdateStarted { uid: &'a String },
    ProfileUpdateCompleted { uid: &'a String },
}

#[derive(Debug)]
pub struct NotificationSystem {}

impl NotificationSystem {
    pub(crate) fn queue_profile_refresh(profile: Option<String>) {
        static QUEUE: OnceLock<ProfileRefreshQueue> = OnceLock::new();
        QUEUE.get_or_init(ProfileRefreshQueue::start).enqueue(profile);
    }
    fn emit_to_window(window: &WebviewWindow, event: FrontendEvent) {
        let (event_name, Ok(payload)) = Self::serialize_event(event) else {
            return;
        };

        if let Err(e) = window.emit(event_name, payload) {
            logging!(warn, Type::Frontend, "Event emit failed: {}", e);
        }
    }

    fn serialize_event(event: FrontendEvent) -> (&'static str, Result<serde_json::Value, serde_json::Error>) {
        match event {
            FrontendEvent::RefreshClash => ("verge://refresh-clash-config", Ok(json!("yes"))),
            FrontendEvent::RefreshVerge => ("verge://refresh-verge-config", Ok(json!("yes"))),
            FrontendEvent::NoticeMessage { status, message } => {
                ("verge://notice-message", serde_json::to_value((status, message)))
            }
            FrontendEvent::ProfileChanged { current_profile_id } => ("profile-changed", Ok(json!(current_profile_id))),
            FrontendEvent::TimerUpdated { profile_index } => ("verge://timer-updated", Ok(json!(profile_index))),
            FrontendEvent::ProfileUpdateStarted { uid } => ("profile-update-started", Ok(json!({ "uid": uid }))),
            FrontendEvent::ProfileUpdateCompleted { uid } => ("profile-update-completed", Ok(json!({ "uid": uid }))),
        }
    }

    pub(crate) fn send_event(event: FrontendEvent) {
        if let Some(window) = WindowManager::get_main_window() {
            Self::emit_to_window(&window, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_consumer_coalesces_270_switches_to_latest_profile() {
        let pending = Arc::new(PendingProfileRefresh::default());
        let (wake, receiver) = mpsc::sync_channel(1);
        let queue = ProfileRefreshQueue {
            pending: pending.clone(),
            wake,
        };
        for index in 0..270 {
            queue.enqueue(Some(format!("profile-{index}").into()));
        }
        assert_eq!(pending.take(), Some(Some("profile-269".into())));
        assert!(pending.take().is_none());
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
        queue.enqueue(Some("after-recovery".into()));
        assert!(receiver.try_recv().is_ok());
        assert_eq!(pending.take(), Some(Some("after-recovery".into())));
    }
}
