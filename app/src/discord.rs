use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use std::{
    sync::mpsc::{self, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};

/// Discord application used for Rich Presence.
///
/// This must be a SENZO-owned application from the Discord Developer Portal.
/// The build sets `SENZO_DISCORD_APP_ID`; an unset value disables presence
/// instead of borrowing another application's identity.
const DISCORD_APP_ID: Option<&str> = option_env!("SENZO_DISCORD_APP_ID");

/// Uploaded Rich Presence asset key, or an external HTTPS image. The
/// fallback is intentionally SENZO-branded; `stremio_logo` was removed.
const SENZO_FALLBACK_IMAGE: &str = match option_env!("SENZO_DISCORD_FALLBACK_IMAGE") {
    Some(image) => image,
    None => DEFAULT_SENZO_FALLBACK_IMAGE,
};
const DEFAULT_SENZO_FALLBACK_IMAGE: &str = "https://raw.githubusercontent.com/Sensei002/senzo/development/app/ui/assets/images/senzo-logo.png";

fn fallback_image() -> &'static str {
    let configured = SENZO_FALLBACK_IMAGE.trim();
    if configured.is_empty() {
        DEFAULT_SENZO_FALLBACK_IMAGE
    } else {
        configured
    }
}

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

fn configured_app_id() -> Option<&'static str> {
    const STREMIO_APP_ID: &str = "1452620752263319665";
    let app_id = DISCORD_APP_ID?.trim();
    if app_id.is_empty() || app_id == STREMIO_APP_ID {
        None
    } else {
        Some(app_id)
    }
}

#[derive(Clone)]
struct ActivityState {
    state: String,
    details: String,
    large_image: Option<String>,
    start_timestamp: Option<i64>,
    end_timestamp: Option<i64>,
}

enum DiscordCommand {
    Connect,
    Disconnect,
    SetActivity(ActivityState),
    ClearActivity,
}

struct DiscordWorker {
    client: Option<DiscordIpcClient>,
    enabled: bool,
    desired_activity: Option<ActivityState>,
    next_retry_at: Instant,
    retry_delay: Duration,
    failure_logged: bool,
    configuration_logged: bool,
}

impl DiscordWorker {
    fn new() -> Self {
        Self {
            client: None,
            enabled: false,
            desired_activity: None,
            next_retry_at: Instant::now(),
            retry_delay: INITIAL_RETRY_DELAY,
            failure_logged: false,
            configuration_logged: false,
        }
    }

    fn handle(&mut self, command: DiscordCommand) {
        match command {
            DiscordCommand::Connect => {
                let was_enabled = self.enabled;
                self.enabled = true;
                self.failure_logged = false;
                if !was_enabled {
                    self.next_retry_at = Instant::now();
                }
                self.connect_if_due(!was_enabled);
            }
            DiscordCommand::Disconnect => self.disconnect(),
            DiscordCommand::SetActivity(activity) => {
                self.desired_activity = Some(activity.clone());
                if self.client.is_some() {
                    self.publish_activity(&activity);
                } else {
                    // A media, pause, or resume transition is also a reconnect
                    // opportunity. The deadline prevents rapid state bursts from
                    // hammering Discord's IPC endpoint.
                    self.connect_if_due(false);
                }
            }
            DiscordCommand::ClearActivity => {
                self.desired_activity = None;
                let Some(current_client) = self.client.as_mut() else {
                    return;
                };
                if let Err(error) = current_client.clear_activity() {
                    tracing::error!(%error, "Discord RPC clear activity failed");
                    self.mark_connection_lost();
                }
            }
        }
    }

    fn retry_wait(&self) -> Option<Duration> {
        (self.enabled && self.client.is_none())
            .then(|| self.next_retry_at.saturating_duration_since(Instant::now()))
    }

    fn connect_if_due(&mut self, force: bool) {
        if !self.enabled || self.client.is_some() {
            return;
        }
        if !force && Instant::now() < self.next_retry_at {
            return;
        }

        let Some(app_id) = configured_app_id() else {
            if !self.configuration_logged {
                self.configuration_logged = true;
                tracing::warn!(
                    "Discord Rich Presence is disabled: SENZO_DISCORD_APP_ID is not configured with a SENZO application ID"
                );
            }
            self.enabled = false;
            return;
        };

        let mut next_client = DiscordIpcClient::new(app_id);
        match next_client.connect() {
            Ok(()) => {
                tracing::info!("Discord RPC connected successfully");
                self.client = Some(next_client);
                self.retry_delay = INITIAL_RETRY_DELAY;
                self.next_retry_at = Instant::now();
                self.failure_logged = false;
                if let Some(activity) = self.desired_activity.clone() {
                    self.publish_activity(&activity);
                }
            }
            Err(error) => {
                let retry_delay = self.schedule_retry();
                if !self.failure_logged {
                    self.failure_logged = true;
                    tracing::warn!(
                        %error,
                        retry_in_seconds = retry_delay.as_secs(),
                        "Discord RPC connection unavailable; retry scheduled"
                    );
                } else {
                    tracing::debug!(
                        %error,
                        retry_in_seconds = retry_delay.as_secs(),
                        "Discord RPC connection still unavailable"
                    );
                }
            }
        }
    }

    fn publish_activity(&mut self, activity_state: &ActivityState) {
        let Some(current_client) = self.client.as_mut() else {
            return;
        };

        let mut payload = activity::Activity::new()
            .activity_type(activity::ActivityType::Watching)
            .state(&activity_state.state)
            .details(&activity_state.details);
        payload = payload.assets(
            activity::Assets::new()
                .large_image(
                    activity_state
                        .large_image
                        .as_deref()
                        .unwrap_or(fallback_image()),
                )
                .large_text("SENZO"),
        );

        let timestamps = match (activity_state.start_timestamp, activity_state.end_timestamp) {
            (Some(start), Some(end)) => Some(activity::Timestamps::new().start(start).end(end)),
            (Some(start), None) => Some(activity::Timestamps::new().start(start)),
            (None, Some(end)) => Some(activity::Timestamps::new().end(end)),
            (None, None) => None,
        };
        if let Some(timestamps) = timestamps {
            payload = payload.timestamps(timestamps);
        }

        if let Err(error) = current_client.set_activity(payload) {
            tracing::error!(%error, "Discord RPC set activity failed");
            self.mark_connection_lost();
        }
    }

    fn mark_connection_lost(&mut self) {
        self.client = None;
        self.schedule_retry();
    }

    fn schedule_retry(&mut self) -> Duration {
        let delay = self.retry_delay;
        self.next_retry_at = Instant::now() + delay;
        self.retry_delay = self.retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
        delay
    }

    fn disconnect(&mut self) {
        self.enabled = false;
        self.desired_activity = None;
        self.retry_delay = INITIAL_RETRY_DELAY;
        self.next_retry_at = Instant::now();
        if let Some(mut current_client) = self.client.take() {
            tracing::info!("Discord RPC disconnecting");
            let _ = current_client.clear_activity();
            if let Err(error) = current_client.close() {
                tracing::error!(%error, "Discord RPC disconnect failed");
            }
        }
    }
}

pub struct DiscordRpc {
    commands: Sender<DiscordCommand>,
}

impl Default for DiscordRpc {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordRpc {
    pub fn new() -> Self {
        let (commands, receiver) = mpsc::channel();

        std::thread::spawn(move || {
            let mut worker = DiscordWorker::new();
            loop {
                let command = match worker.retry_wait() {
                    Some(wait) => match receiver.recv_timeout(wait) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                    None => match receiver.recv() {
                        Ok(command) => Some(command),
                        Err(_) => break,
                    },
                };
                if let Some(command) = command {
                    worker.handle(command);
                }
                worker.connect_if_due(false);
            }
            worker.disconnect();
        });

        Self { commands }
    }

    pub fn connect(&self) -> Result<(), String> {
        self.send(DiscordCommand::Connect)
    }

    pub fn disconnect(&self) -> Result<(), String> {
        self.send(DiscordCommand::Disconnect)
    }

    pub fn set_activity(
        &self,
        state: &str,
        details: &str,
        large_image: Option<&str>,
        start_timestamp: Option<i64>,
        end_timestamp: Option<i64>,
    ) -> Result<(), String> {
        self.send(DiscordCommand::SetActivity(ActivityState {
            state: state.to_string(),
            details: details.to_string(),
            large_image: large_image.map(ToString::to_string),
            start_timestamp,
            end_timestamp,
        }))
    }

    pub fn clear_activity(&self) -> Result<(), String> {
        self.send(DiscordCommand::ClearActivity)
    }

    fn send(&self, command: DiscordCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|e| format!("Failed to send Discord command: {e}"))
    }
}

impl Drop for DiscordRpc {
    fn drop(&mut self) {
        let _ = self.commands.send(DiscordCommand::Disconnect);
    }
}
