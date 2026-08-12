from pathlib import Path

from PIL import Image


ICON_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)


def load_icon_images(icon_dir: Path) -> list[Image.Image]:
    """Load the PNG icon sources in ascending size order."""
    return [
        Image.open(icon_dir / f"{size}x{size}.png").convert("RGBA")
        for size in ICON_SIZES
    ]


def rebuild_ico(icon_dir: Path) -> Path:
    """Rebuild icon.ico from the checked-in PNG source sizes."""
    images = load_icon_images(icon_dir)
    output_path = icon_dir / "icon.ico"
    images[-1].save(
        output_path,
        format="ICO",
        sizes=[(size, size) for size in ICON_SIZES],
        append_images=list(reversed(images[:-1])),
    )
    return output_path


def main() -> None:
    """Rebuild the Windows icon beside this script."""
    output_path = rebuild_ico(Path(__file__).resolve().parent)
    print(f"saved {output_path.name}")


if __name__ == "__main__":
    main()
