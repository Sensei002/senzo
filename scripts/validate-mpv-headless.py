#!/usr/bin/env python3
import array
import math
import sys
import wave
from pathlib import Path

from PIL import Image, ImageOps, ImageStat


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate_screen(path: Path) -> None:
    if not path.is_file() or path.stat().st_size < 10_000:
        fail(f"video capture is missing or unexpectedly small: {path}")
    with Image.open(path) as source:
        image = source.convert("RGB")
    gray = ImageOps.grayscale(image)
    stats = ImageStat.Stat(gray)
    mask = gray.point(lambda value: 255 if value > 18 else 0)
    bounds = mask.getbbox()
    if bounds is None:
        fail("captured virtual display is completely black")
    left, top, right, bottom = bounds
    area = (right - left) * (bottom - top)
    if area < 20_000 or stats.stddev[0] < 18:
        fail(
            "captured display does not contain a sufficiently large, "
            f"visually varied video frame (bounds={bounds}, stddev={stats.stddev[0]:.1f})"
        )
    print(
        "PASS: non-black, spatially varied video was captured from the virtual display "
        f"({image.width}x{image.height}, video bounds={bounds}, luma stddev={stats.stddev[0]:.1f})"
    )


def validate_audio(path: Path) -> None:
    if not path.is_file() or path.stat().st_size < 2_000:
        fail(f"virtual audio capture is missing or too small: {path}")
    with wave.open(str(path), "rb") as stream:
        channels = stream.getnchannels()
        rate = stream.getframerate()
        width = stream.getsampwidth()
        frames = stream.getnframes()
        raw = stream.readframes(frames)
    if width != 2:
        fail(f"expected 16-bit PCM from PulseAudio monitor, received {width * 8}-bit")
    samples = array.array("h")
    samples.frombytes(raw)
    if sys.byteorder != "little":
        samples.byteswap()
    if not samples:
        fail("virtual audio capture contains no samples")
    peak = max(abs(value) for value in samples)
    rms = math.sqrt(sum(value * value for value in samples) / len(samples))
    duration = frames / rate if rate else 0.0
    if duration < 2.0 or peak < 300 or rms < 50:
        fail(
            "virtual audio output is silent or too short "
            f"(duration={duration:.2f}s, peak={peak}, rms={rms:.1f})"
        )
    print(
        "PASS: non-silent PCM was captured from the virtual audio sink "
        f"({duration:.2f}s, {rate}Hz, {channels} channel(s), peak={peak}, rms={rms:.1f})"
    )


if __name__ == "__main__":
    if len(sys.argv) != 3:
        fail("usage: validate-mpv-headless.py <captured-frame.png> <audio.wav>")
    validate_screen(Path(sys.argv[1]))
    validate_audio(Path(sys.argv[2]))
