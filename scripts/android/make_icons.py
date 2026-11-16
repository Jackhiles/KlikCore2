"""Make the Android launcher icons (android/res/mipmap-*/ic_launcher.png) from
assets/icon/klikcore-256.png. Needs Pillow; run it again when the icon changes.
The APK build (build.py) only uses the PNGs this writes."""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
SIZES = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}


def main():
    src = Image.open(ROOT / "assets" / "icon" / "klikcore-256.png").convert("RGBA")
    for density, px in SIZES.items():
        out = ROOT / "android" / "res" / f"mipmap-{density}" / "ic_launcher.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        src.resize((px, px), Image.LANCZOS).save(out, optimize=True)
        print(out.relative_to(ROOT))


if __name__ == "__main__":
    main()
