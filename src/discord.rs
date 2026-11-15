//! Optional Discord Rich Presence over the local Discord IPC named pipe.
//!
//! Presence is opt-in and never sends media titles unless the user separately enables
//! title sharing. No network requests are made by SENZO; communication stays local.

use std::{
    sync::mpsc::{self, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const RETRY_DELAY: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
struct PlaybackSnapshot {
    title: Option<String>,
    media_type: Option<String>,
    paused: bool,
    position_seconds: Option<f64>,
    duration_seconds: Option<f64>,
}

enum Command {
    Configure { enabled: bool, share_titles: bool },
    Playback(PlaybackSnapshot),
    ClearPlayback,
    Shutdown,
}

pub struct DiscordPresence {
    sender: Sender<Command>,
}

impl DiscordPresence {
    pub fn new(enabled: bool, share_titles: bool) -> Self {
        let (sender, receiver) = mpsc::channel();
        let client_id = option_env!("SENZO_DISCORD_CLIENT_ID")
            .unwrap_or_default()
            .trim()
            .to_owned();
        let _ = thread::Builder::new()
            .name("senzo-discord-presence".into())
            .spawn(move || run_presence_worker(receiver, client_id, enabled, share_titles));
        Self { sender }
    }

    pub fn is_configured() -> bool {
        option_env!("SENZO_DISCORD_CLIENT_ID")
            .is_some_and(|value| !value.trim().is_empty() && value.trim().bytes().all(|b| b.is_ascii_digit()))
    }

    pub fn configure(&self, enabled: bool, share_titles: bool) {
        let _ = self.sender.send(Command::Configure { enabled, share_titles });
    }

    pub fn clear_playback(&self) {
        let _ = self.sender.send(Command::ClearPlayback);
    }

    pub fn update_playback(
        &self,
        title: Option<String>,
        media_type: Option<String>,
        paused: bool,
        position_seconds: Option<f64>,
        duration_seconds: Option<f64>,
    ) {
        let _ = self.sender.send(Command::Playback(PlaybackSnapshot {
            title,
            media_type,
            paused,
            position_seconds,
            duration_seconds,
        }));
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
    }
}

fn run_presence_worker(
    receiver: mpsc::Receiver<Command>,
    client_id: String,
    mut enabled: bool,
    mut share_titles: bool,
) {
    let mut playback: Option<PlaybackSnapshot> = None;
    let mut pipe = None;
    let mut last_activity = String::new();
    let mut retry_at = Instant::now();

    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(Command::Configure { enabled: next_enabled, share_titles: next_share_titles }) => {
                enabled = next_enabled;
                share_titles = next_share_titles;
                last_activity.clear();
                if !enabled {
                    if let Some(mut current) = pipe.take() {
                        let _ = write_activity(&mut current, None);
                    }
                }
            }
            Ok(Command::Playback(snapshot)) => {
                playback = Some(snapshot);
            }
            Ok(Command::ClearPlayback) => {
                playback = None;
            }
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(mut current) = pipe.take() {
                    let _ = write_activity(&mut current, None);
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if !enabled || client_id.is_empty() {
            continue;
        }

        if pipe.is_none() && Instant::now() >= retry_at {
            pipe = connect_discord(&client_id);
            if pipe.is_none() {
                retry_at = Instant::now() + RETRY_DELAY;
                continue;
            }
            last_activity.clear();
        }

        let Some(current) = pipe.as_mut() else {
            continue;
        };
        let activity = activity_payload(playback.as_ref(), share_titles);
        let mut activity_key = activity.clone();
        if let Some(object) = activity_key.as_object_mut() {
            object.remove("timestamps");
        }
        let encoded = activity_key.to_string();
        if encoded == last_activity {
            continue;
        }
        if write_activity(current, Some(activity)).is_ok() {
            last_activity = encoded;
        } else {
            pipe = None;
            retry_at = Instant::now() + RETRY_DELAY;
            last_activity.clear();
        }
    }
}

fn activity_payload(snapshot: Option<&PlaybackSnapshot>, share_titles: bool) -> Value {
    let Some(snapshot) = snapshot else {
        return json!({
            "type": 3,
            "details": "Browsing SENZO"
        });
    };

    let kind = match snapshot.media_type.as_deref() {
        Some("tv") | Some("series") => "a TV show",
        Some("movie") => "a movie",
        _ => "media",
    };
    let details = if share_titles {
        snapshot.title.as_deref().filter(|title| !title.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Watching {kind}"))
    } else {
        format!("Watching {kind}")
    };
    let mut activity = json!({
        "type": 3,
        "details": details,
        "state": if snapshot.paused { "Paused" } else { "Playing" }
    });

    if !snapshot.paused {
        if let (Some(position), Some(duration)) =
            (snapshot.position_seconds, snapshot.duration_seconds)
        {
            if position.is_finite() && duration.is_finite() && duration > 0.0 {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let start = now.saturating_sub(position.max(0.0) as i64);
                let end = start.saturating_add(duration as i64);
                activity["timestamps"] = json!({ "start": start, "end": end });
            }
        }
    }
    activity
}

#[cfg(windows)]
fn connect_discord(client_id: &str) -> Option<std::fs::File> {
    use std::{
        fs::OpenOptions,
        io::{Read, Write},
    };

    for index in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        let Ok(mut file) = OpenOptions::new().read(true).write(true).open(path) else {
            continue;
        };
        let handshake = json!({ "v": 1, "client_id": client_id }).to_string();
        if write_frame(&mut file, 0, handshake.as_bytes()).is_err() {
            continue;
        }
        let mut header = [0_u8; 8];
        if file.read_exact(&mut header).is_err() {
            continue;
        }
        let opcode = u32::from_le_bytes(header[0..4].try_into().ok()?);
        let length = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        if opcode != 1 || length > 1_048_576 {
            continue;
        }
        let mut payload = vec![0_u8; length];
        if file.read_exact(&mut payload).is_err() {
            continue;
        }
        let ready: Value = serde_json::from_slice(&payload).ok()?;
        if ready.get("evt").and_then(Value::as_str) == Some("READY") {
            return Some(file);
        }
        let _ = file.flush();
    }
    None
}

#[cfg(not(windows))]
fn connect_discord(_client_id: &str) -> Option<std::fs::File> {
    None
}

#[cfg(windows)]
fn write_activity(file: &mut std::fs::File, activity: Option<Value>) -> std::io::Result<()> {
    use std::io::{Read, Write};
    use uuid::Uuid;

    let payload = json!({
        "cmd": "SET_ACTIVITY",
        "nonce": Uuid::new_v4().to_string(),
        "args": {
            "pid": std::process::id(),
            "activity": activity
        }
    });
    let encoded = payload.to_string();
    write_frame(file, 1, encoded.as_bytes())?;
    file.flush()?;

    let mut header = [0_u8; 8];
    file.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap_or([0; 4]));
    let length = u32::from_le_bytes(header[4..8].try_into().unwrap_or([0; 4])) as usize;
    if opcode != 1 || length > 1_048_576 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid Discord IPC response"));
    }
    let mut response = vec![0_u8; length];
    file.read_exact(&mut response)?;
    let response: Value = serde_json::from_slice(&response)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if response.get("cmd").and_then(Value::as_str) != Some("SET_ACTIVITY") {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unexpected Discord IPC command response"));
    }
    Ok(())
}

#[cfg(windows)]
fn write_frame(file: &mut std::fs::File, opcode: u32, payload: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let length = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Discord IPC payload too large"))?;
    file.write_all(&opcode.to_le_bytes())?;
    file.write_all(&length.to_le_bytes())?;
    file.write_all(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_hides_title_by_default() {
        let snapshot = PlaybackSnapshot {
            title: Some("Private Movie".into()),
            media_type: Some("movie".into()),
            paused: false,
            position_seconds: Some(30.0),
            duration_seconds: Some(120.0),
        };
        let activity = activity_payload(Some(&snapshot), false);
        assert_eq!(activity["details"], "Watching a movie");
        assert_ne!(activity["details"], "Private Movie");
    }

    #[test]
    fn title_sharing_is_explicit_and_paused_state_has_no_progress_timer() {
        let snapshot = PlaybackSnapshot {
            title: Some("Example Episode".into()),
            media_type: Some("series".into()),
            paused: true,
            position_seconds: Some(30.0),
            duration_seconds: Some(120.0),
        };
        let activity = activity_payload(Some(&snapshot), true);
        assert_eq!(activity["details"], "Example Episode");
        assert_eq!(activity["state"], "Paused");
        assert!(activity.get("timestamps").is_none());
    }

    #[test]
    fn idle_presence_uses_generic_application_label() {
        assert_eq!(activity_payload(None, true)["details"], "Browsing SENZO");
    }
}
