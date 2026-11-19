#!/usr/bin/env bash
set -euo pipefail

ROOT="$(pwd)"
ARTIFACT_DIR="$ROOT/.artifacts/mpv-headless-av"
TMP_DIR="$(mktemp -d)"
FRAME_DIR="$ARTIFACT_DIR/frames"
mkdir -p "$FRAME_DIR" "$ARTIFACT_DIR"
cleanup() {
  for pid in "${capture_pid:-}" "${audio_pid:-}" "${server_pid:-}"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT

echo "Starting PulseAudio and creating a virtual output sink"
pulseaudio --start --exit-idle-time=-1
for attempt in $(seq 1 20); do
  if pactl info >/dev/null 2>&1; then break; fi
  sleep 0.25
done
pactl info >/dev/null
pactl load-module module-null-sink sink_name=senzo_test rate=48000 channels=2
pactl set-default-sink senzo_test

echo "Generating a seekable 60-second H.264/AAC fixture"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc=size=640x360:rate=30" \
  -f lavfi -i "sine=frequency=880:sample_rate=48000" \
  -t 60 -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
  -c:a aac -b:a 128k -movflags +faststart \
  "$TMP_DIR/fixture.mp4"

python3 scripts/mpv-range-server.py --directory "$TMP_DIR" --port 18765 \
  >"$ARTIFACT_DIR/http-server.log" 2>&1 &
server_pid=$!
for attempt in $(seq 1 40); do
  if curl -fsS -I http://127.0.0.1:18765/fixture.mp4 >/dev/null; then break; fi
  sleep 0.25
done
curl -fsS -I http://127.0.0.1:18765/fixture.mp4 >/dev/null

echo "Capturing virtual display frames and virtual speaker output"
ffmpeg -hide_banner -loglevel error -y -f x11grab -framerate 1 \
  -video_size 1280x720 -i "$DISPLAY" -frames:v 40 \
  "$FRAME_DIR/frame-%03d.png" >"$ARTIFACT_DIR/frame-capture.log" 2>&1 &
capture_pid=$!
parec --device=senzo_test.monitor --format=s16le --rate=48000 --channels=2 \
  >"$ARTIFACT_DIR/audio.pcm" 2>"$ARTIFACT_DIR/audio-capture.log" &
audio_pid=$!
sleep 1

echo "Running SENZO's libmpv acceptance sequence against the generated fixture"
LIBGL_ALWAYS_SOFTWARE=1 target/release/senzo \
  --mpv-headless-test http://127.0.0.1:18765/fixture.mp4 \
  2>&1 | tee "$ARTIFACT_DIR/player-acceptance.log"

sleep 1
kill "$capture_pid" "$audio_pid" 2>/dev/null || true
wait "$capture_pid" 2>/dev/null || true
wait "$audio_pid" 2>/dev/null || true
unset capture_pid audio_pid

echo "Verifying actual captured video and audio output"
python3 scripts/verify-mpv-av.py \
  --frames "$FRAME_DIR" \
  --audio "$ARTIFACT_DIR/audio.pcm" \
  --report "$ARTIFACT_DIR/verification.json"
