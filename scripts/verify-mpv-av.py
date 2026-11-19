#!/usr/bin/env python3
"""Verify captured virtual-display frames and PulseAudio PCM from MPV CI."""
from __future__ import annotations

import argparse
import json
import math
import struct
from pathlib import Path

from PIL import Image, ImageChops, ImageStat


def inspect_frames(directory: Path) -> dict[str, object]:
    paths = sorted(directory.glob("frame-*.png"))
    active: list[Image.Image] = []
    for path in paths:
        with Image.open(path) as source:
            image = source.convert("RGB").resize((96, 54))
        pixels = list(image.getdata())
        bright = sum(1 for pixel in pixels if max(pixel) > 25)
        if bright / len(pixels) > 0.01:
            active.append(image)

    motion_scores: list[float] = []
    for before, after in zip(active, active[1:]):
        difference = ImageChops.difference(before, after).convert("L")
        motion_scores.append(ImageStat.Stat(difference).mean[0])

    maximum_motion = max(motion_scores, default=0.0)
    result = {
        "captured_frames": len(paths),
        "frames_with_visible_content": len(active),
        "maximum_frame_difference_mean": maximum_motion,
    }
    if len(active) < 3:
        raise SystemExit(f"FAIL: expected at least 3 visible video frames; got {result}")
    if maximum_motion < 0.8:
        raise SystemExit(f"FAIL: captured video frames did not show motion; got {result}")
    return result


def inspect_audio(path: Path) -> dict[str, object]:
    raw = path.read_bytes()
    if len(raw) < 8192 or len(raw) % 2:
        raise SystemExit(f"FAIL: PulseAudio capture is missing or too small ({len(raw)} bytes)")
    count = len(raw) // 2
    squares = 0
    peak = 0
    for (sample,) in struct.iter_unpack("<h", raw):
        value = abs(sample)
        peak = max(peak, value)
        squares += sample * sample
    rms = math.sqrt(squares / count)
    result = {
        "pcm_bytes": len(raw),
        "sample_count": count,
        "peak": peak,
        "rms": rms,
    }
    if peak < 100 or rms < 20:
        raise SystemExit(f"FAIL: virtual audio capture is silent; got {result}")
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--frames", type=Path, required=True)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()

    report = {
        "video": inspect_frames(args.frames),
        "audio": inspect_audio(args.audio),
        "result": "passed",
        "scope": "libmpv rendered to Xvfb and audio routed to PulseAudio virtual sink; not physical-device proof",
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
