#!/usr/bin/env bash
set -euo pipefail

PLAYER_BIN="$1"
OUT="/tmp/senzo-mpv-headless"
MEDIA="$OUT/media"
mkdir -p "$MEDIA"
rm -f "$OUT/audio.wav" "$OUT/screen.mkv" "$OUT/screen.png"
HTTP_PID=""
AUDIO_PID=""
SCREEN_PID=""

cleanup() {
  if [ -n "$HTTP_PID" ]; then kill "$HTTP_PID" 2>/dev/null || true; fi
  if [ -n "$AUDIO_PID" ]; then kill -INT "$AUDIO_PID" 2>/dev/null || true; fi
  if [ -n "$SCREEN_PID" ]; then kill -INT "$SCREEN_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

echo "Generating deterministic audiovisual fixtures"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=640x360:rate=30" \
  -f lavfi -i "sine=frequency=440:sample_rate=48000" \
  -t 45 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 30 \
  -c:a aac -b:a 128k -shortest -movflags +faststart "$MEDIA/first.mp4"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "smptebars=size=640x360:rate=30" \
  -f lavfi -i "sine=frequency=880:sample_rate=48000" \
  -t 45 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 30 \
  -c:a aac -b:a 128k -shortest -movflags +faststart "$MEDIA/second.mp4"

echo "Starting PulseAudio with a virtual monitorable sink"
pulseaudio --start --exit-idle-time=-1 --log-target="file:$OUT/pulseaudio.log"
for _ in $(seq 1 30); do
  if pactl info >/dev/null 2>&1; then break; fi
  sleep 1
done
pactl info >/dev/null
pactl load-module module-null-sink sink_name=senzo_headless_sink sink_properties=device.description=SENZO_Headless_Test >/dev/null
pactl set-default-sink senzo_headless_sink
parec --device=senzo_headless_sink.monitor --file-format=wav "$OUT/audio.wav" >"$OUT/parec.log" 2>&1 &
AUDIO_PID=$!

python3 scripts/mpv-range-server.py --directory "$MEDIA" --port 18765 >"$OUT/http.log" 2>&1 &
HTTP_PID=$!
for _ in $(seq 1 30); do
  if curl --fail --silent --head http://127.0.0.1:18765/first.mp4 >/dev/null; then break; fi
  sleep 1
done
curl --fail --silent --head http://127.0.0.1:18765/first.mp4 >/dev/null
curl --fail --silent --range 0-1023 --output /dev/null --write-out '%{http_code}' http://127.0.0.1:18765/first.mp4 | grep -qx 206

echo "Capturing the X11 display while SENZO drives libmpv"
ffmpeg -hide_banner -loglevel warning -y \
  -f x11grab -framerate 8 -video_size 1280x720 -i "$DISPLAY" \
  -t 16 -an -c:v ffv1 "$OUT/screen.mkv" >"$OUT/screen-capture.log" 2>&1 &
SCREEN_PID=$!

echo "Running the real SENZO libmpv playback/control integration"
export SENZO_MPV_LOG_FILE="$OUT/mpv.log"
PULSE_SINK=senzo_headless_sink timeout 100 "$PLAYER_BIN" \
  "http://127.0.0.1:18765/first.mp4" \
  "http://127.0.0.1:18765/second.mp4"

echo "Stopping audio capture and finalizing display recording"
kill -INT "$AUDIO_PID" 2>/dev/null || true
wait "$AUDIO_PID" 2>/dev/null || true
AUDIO_PID=""
wait "$SCREEN_PID"
SCREEN_PID=""
kill "$HTTP_PID" 2>/dev/null || true
HTTP_PID=""

ffmpeg -hide_banner -loglevel error -y -ss 3 -i "$OUT/screen.mkv" -frames:v 1 "$OUT/screen.png"
python3 scripts/validate-mpv-headless.py "$OUT/screen.png" "$OUT/audio.wav"
echo "PASS: SENZO headless audiovisual acceptance completed"
