use senzo_player::{MpvConfig, MpvPlayer, PlaybackSnapshot, PlaybackStatus, TrackKind};
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    thread,
    time::{Duration, Instant},
};

fn wait_for(
    player: &MpvPlayer,
    label: &str,
    timeout: Duration,
    predicate: impl Fn(&PlaybackSnapshot) -> bool,
) -> Result<PlaybackSnapshot, Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = player.snapshot();
        if predicate(&snapshot) {
            println!("PASS: {label}");
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(
                format!("timed out waiting for {label}; last snapshot: {snapshot:?}").into(),
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let first = args.next().ok_or("first media URL is required")?;
    let second = args.next().ok_or("replacement media URL is required")?;
    if args.next().is_some() {
        return Err("unexpected extra arguments".into());
    }

    let player = MpvPlayer::spawn(MpvConfig {
        video_output: Some("gpu".into()),
        audio_output: Some("pulse".into()),
        force_window: Some(true),
        gpu_api: Some("opengl".into()),
        gpu_context: Some("x11egl".into()),
        ..MpvConfig::default()
    })?;
    let headers = BTreeMap::new();

    println!("Loading first generated HTTP fixture");
    player.load(&first, &headers)?;
    let initial = wait_for(
        &player,
        "first source decodes video and audio",
        Duration::from_secs(20),
        |s| {
            s.status == PlaybackStatus::Playing
                && s.video_width.is_some_and(|v| v > 0)
                && s.video_height.is_some_and(|v| v > 0)
                && s.video_codec.is_some()
                && s.audio_codec.is_some()
                && s.tracks
                    .iter()
                    .any(|t| t.kind == TrackKind::Audio && t.selected)
        },
    )?;
    println!(
        "First stream: {}x{}, video={:?}, audio={:?}, duration={:?}",
        initial.video_width.unwrap_or_default(),
        initial.video_height.unwrap_or_default(),
        initial.video_codec,
        initial.audio_codec,
        initial.duration_seconds
    );

    wait_for(
        &player,
        "playback clock advances",
        Duration::from_secs(8),
        |s| s.position_seconds.unwrap_or(0.0) > 1.0,
    )?;

    let audio_id = initial
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio && t.selected)
        .map(|t| t.id)
        .ok_or("decoded stream has no selected audio track")?;
    player.select_audio_track(Some(audio_id))?;
    player.set_volume(63.0)?;
    wait_for(
        &player,
        "volume control is applied",
        Duration::from_secs(3),
        |s| (s.volume_percent - 63.0).abs() < 1.0,
    )?;
    player.set_muted(true)?;
    wait_for(
        &player,
        "mute control is applied",
        Duration::from_secs(3),
        |s| s.muted,
    )?;
    player.set_muted(false)?;
    wait_for(
        &player,
        "unmute control is applied",
        Duration::from_secs(3),
        |s| !s.muted,
    )?;
    player.set_speed(1.25)?;
    wait_for(
        &player,
        "playback speed control is applied",
        Duration::from_secs(3),
        |s| (s.speed - 1.25).abs() < 0.02,
    )?;
    player.set_speed(1.0)?;

    player.pause()?;
    wait_for(
        &player,
        "pause control reaches Paused state",
        Duration::from_secs(3),
        |s| s.status == PlaybackStatus::Paused && s.paused,
    )?;
    player.seek_absolute(5.0)?;
    wait_for(
        &player,
        "absolute seek updates the paused playback position",
        Duration::from_secs(5),
        |s| {
            s.status == PlaybackStatus::Paused
                && s.position_seconds.is_some_and(|p| (4.7..=5.5).contains(&p))
        },
    )?;
    player.play()?;
    wait_for(
        &player,
        "resume advances playback after seek",
        Duration::from_secs(5),
        |s| s.status == PlaybackStatus::Playing && s.position_seconds.unwrap_or(0.0) > 5.5,
    )?;

    println!("Replacing the source while keeping the same MPV owner alive");
    player.load(&second, &headers)?;
    wait_for(
        &player,
        "replacement source starts and exposes its video/audio streams",
        Duration::from_secs(15),
        |s| {
            s.status == PlaybackStatus::Playing
                && s.media_title
                    .as_deref()
                    .is_some_and(|title| title.contains("second"))
                && s.video_width.is_some_and(|v| v > 0)
                && s.audio_codec.is_some()
        },
    )?;

    player.stop(Duration::from_secs(10))?;
    wait_for(
        &player,
        "stop acknowledgement publishes Stopped",
        Duration::from_secs(3),
        |s| s.status == PlaybackStatus::Stopped,
    )?;

    println!("Starting a second playback session on the same MPV owner");
    player.load(&first, &headers)?;
    wait_for(
        &player,
        "second session starts",
        Duration::from_secs(15),
        |s| {
            s.status == PlaybackStatus::Playing
                && s.media_title
                    .as_deref()
                    .is_some_and(|title| title.contains("first"))
                && s.video_width.is_some_and(|v| v > 0)
                && s.audio_codec.is_some()
        },
    )?;
    player.stop(Duration::from_secs(10))?;
    wait_for(
        &player,
        "second session stops cleanly",
        Duration::from_secs(3),
        |s| s.status == PlaybackStatus::Stopped,
    )?;

    player.shutdown()?;
    println!("PASS: MPV owner thread shuts down cleanly");
    Ok(())
}
