from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets" / "branding" / "senzo-logo.png"
IMAGE_DIR = ROOT / "app" / "ui" / "assets" / "images"
APP_ICON = ROOT / "app" / "assets" / "app.ico"


def main() -> None:
    if not SOURCE.is_file():
        raise SystemExit(f"Canonical SENZO logo not found: {SOURCE}")

    with Image.open(SOURCE) as source:
        logo = source.convert("RGBA")

    if logo.width < 1 or logo.height < 1:
        raise SystemExit("Canonical SENZO logo is empty.")

    IMAGE_DIR.mkdir(parents=True, exist_ok=True)
    APP_ICON.parent.mkdir(parents=True, exist_ok=True)

    # Keep exact source artwork for wide/full-logo placements.
    logo.save(IMAGE_DIR / "logo.png")
    logo.save(IMAGE_DIR / "senzo-logo.png")

    # Generate square application icons by containing the original artwork.
    # This does not crop, distort, or redraw the user's canonical logo.
    size = 512
    canvas = Image.new("RGBA", (size, size), (10, 10, 10, 255))
    contained = logo.copy()
    contained.thumbnail((440, 440), Image.Resampling.LANCZOS)
    offset = ((size - contained.width) // 2, (size - contained.height) // 2)
    canvas.alpha_composite(contained, offset)

    canvas.save(IMAGE_DIR / "app_icon.png", optimize=True)
    canvas.save(
        APP_ICON,
        format="ICO",
        sizes=[
            (16, 16),
            (24, 24),
            (32, 32),
            (48, 48),
            (64, 64),
            (128, 128),
            (256, 256),
        ],
    )


if __name__ == "__main__":
    main()
