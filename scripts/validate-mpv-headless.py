#!/usr/bin/env python3
"""Verify changing video frames and non-silent PCM from SENZO's headless MPV run."""
from __future__ import annotations

import array
import math
import sys
import wave
from pathlib import Path

from PIL import Image, ImageChops, ImageStat


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate_screen(directory: Path) -> None:
    paths = sorted(directory.glob("frame-*.png")) if directory.is_dir() else []
    if len(paths) < 3:
        fail(f"expected at least 3 captured video frames, found {len(paths)}")

    active: list[Image.Image] = []
    for path in paths:
        try:
            with Image.open(path) as source:
                image = source.convert("RGB").resize((96, 54))
        except OSError as error:
            fail(f"could not read captured frame {path}: {error}")
        pixels = list(image.getdata())
        visible = sum(1 for pixel in pixels if max(pixel) > 25)
        if visible / len(pixels) > 0.01:
            active.append(image)

    if len(active) < 3:
        fail(f"expected at least 3 frames containing visible video; found {len(active)}")

    motion_scores = [
        ImageStat.Stat(ImageChops.difference(before, after).convert("L")).mean[0]
        for before, after in zip(active, active[1:])
    ]
    maximum_motion = max(motion_scores, default=0.0)
    if maximum_motion < 0.8:
        fail(
            "captured video frames did not show motion "
            f"(visible frames={len(active)}, max frame difference={maximum_motion:.2f})"
        )

    print(
        "PASS: moving video was captured from the virtual display "
        f"({len(paths)} frames, {len(active)} with visible content, "
        f"maximum frame difference={maximum_motion:.2f})"
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
        fail("usage: validate-mpv-headless.py <frame-directory> <audio.wav>")
    validate_screen(Path(sys.argv[1]))
    validate_audio(Path(sys.argv[2]))
