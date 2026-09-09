//! Advisory desktop state, never a capture lifecycle control or liveness proof.
use std::time::{Duration, Instant};

#[derive(Clone, serde::Serialize)]
pub struct DesktopSnapshot {
    pub session_lock: &'static str,
    pub unlock_generation: u64,
}

#[derive(Default)]
pub struct DesktopState {
    observed: Option<(Instant, bool)>,
    locked_session: Option<String>,
    generation: u64,
}

impl DesktopState {
    pub fn update(&mut self, observation: Option<(String, bool)>, now: Instant) {
        self.observed = observation.as_ref().map(|(_, locked)| (now, *locked));
        if let Some((session, locked)) = observation {
            if locked {
                self.locked_session = Some(session);
            } else {
                if self.locked_session.as_ref() == Some(&session) {
                    self.generation = self.generation.saturating_add(1);
                }
                self.locked_session = None;
            }
        }
    }

    pub fn snapshot(&self, now: Instant) -> DesktopSnapshot {
        let session_lock = match self.observed {
            Some((at, locked)) if now.saturating_duration_since(at) <= Duration::from_secs(6) => {
                if locked {
                    "locked"
                } else {
                    "unlocked"
                }
            }
            _ => "unknown",
        };
        DesktopSnapshot {
            session_lock,
            unlock_generation: self.generation,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_session(text: &str, uid: u32) -> Option<(String, bool)> {
    let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(key));
    if field("User=")?.parse::<u32>().ok()? != uid || !matches!(field("Type=")?, "x11" | "wayland") {
        return None;
    }
    let id = field("Id=")?;
    if id.is_empty() {
        return None;
    }
    let locked = match field("LockedHint=")? {
        "yes" => true,
        "no" => false,
        _ => return None,
    };
    Some((id.to_owned(), locked))
}

pub async fn observe() -> Option<(String, bool)> {
    #[cfg(target_os = "linux")]
    {
        let mut command = tokio::process::Command::new("loginctl");
        command
            .args([
                "show-session",
                "auto",
                "--no-pager",
                "-p",
                "Id",
                "-p",
                "User",
                "-p",
                "Type",
                "-p",
                "LockedHint",
            ])
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_millis(1500), command.output())
            .await
            .ok()?
            .ok()?;
        if !output.status.success() {
            return None;
        }
        // Only the current user's graphical session can suppress UI warnings.
        let uid = unsafe { libc::geteuid() };
        return parse_session(std::str::from_utf8(&output.stdout).ok()?, uid);
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_owned_graphical_session_and_explicit_hint() {
        let text = "Id=1\nUser=1000\nType=x11\nLockedHint=yes\n";
        assert_eq!(parse_session(text, 1000), Some(("1".into(), true)));
        assert!(parse_session(text, 1001).is_none());
        assert!(parse_session(&text.replace("x11", "tty"), 1000).is_none());
        assert!(parse_session(&text.replace("yes", "maybe"), 1000).is_none());
        assert!(parse_session("", 1000).is_none());
    }
    #[test]
    fn expires_lock_evidence_and_preserves_unlock_across_unknown() {
        let mut state = DesktopState::default();
        let now = Instant::now();
        state.update(Some(("1".into(), true)), now);
        assert_eq!(state.snapshot(now).session_lock, "locked");
        assert_eq!(state.snapshot(now + Duration::from_secs(7)).session_lock, "unknown");
        state.update(None, now);
        assert_eq!(state.snapshot(now).session_lock, "unknown");
        state.update(Some(("1".into(), false)), now);
        assert_eq!(state.snapshot(now).unlock_generation, 1);
        state.update(Some(("1".into(), false)), now);
        assert_eq!(state.snapshot(now).unlock_generation, 1);
    }
    #[test]
    fn another_session_is_not_an_unlock() {
        let mut state = DesktopState::default();
        let now = Instant::now();
        state.update(Some(("1".into(), true)), now);
        state.update(Some(("2".into(), false)), now);
        assert_eq!(state.snapshot(now).unlock_generation, 0);
    }
}
