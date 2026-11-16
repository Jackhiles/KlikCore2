"""Draw KlikCore 2's icon: a mint hexagon "core" with a light centre on a dark rounded
tile, and a "2" badge in the corner. Writes klikcore.svg (the design), klikcore-256.png
and klikcore.ico (16-256 px) next to this script. Needs Pillow: `python make_icon.py`."""

import math
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
FONT = HERE.parent / "fonts" / "Inter-SemiBold.ttf"

TILE = (22, 22, 30)
MINT_TOP, MINT_BOTTOM = (90, 240, 190), (30, 190, 150)
LIGHT = (245, 247, 255)
BADGE_TEXT = (16, 40, 34)

# Design units: a 256 x 256 box.
CENTRE = (128.0, 122.0)
HEX_OUTER, HEX_INNER, CORE_R = 84.0, 53.0, 27.0
BADGE_C, BADGE_R, BADGE_RING = (196.0, 196.0), 34.0, 8.0


def hexagon(r):
    return [(CENTRE[0] + r * math.cos(math.radians(60 * k - 90)), CENTRE[1] + r * math.sin(math.radians(60 * k - 90))) for k in range(6)]


def gradient(s, top, bottom):
    img = Image.new("RGBA", (s, s))
    d = ImageDraw.Draw(img)
    for y in range(s):
        t = y / max(s - 1, 1)
        d.line([(0, y), (s, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    return img


def render(size):
    s = size * 4  # supersampled for smooth edges
    u = s / 256
    sc = lambda pts: [(x * u, y * u) for x, y in pts]
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.22), fill=TILE + (255,))
    # Hexagon ring in mint, its middle back to the tile colour, then the core.
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).polygon(sc(hexagon(HEX_OUTER)), fill=255)
    img.paste(gradient(s, MINT_TOP, MINT_BOTTOM), (0, 0), mask)
    d.polygon(sc(hexagon(HEX_INNER)), fill=TILE + (255,))
    cx, cy = CENTRE[0] * u, CENTRE[1] * u
    d.ellipse([cx - CORE_R * u, cy - CORE_R * u, cx + CORE_R * u, cy + CORE_R * u], fill=LIGHT + (255,))
    # "2" badge: a tile-coloured ring keeps it apart from the hexagon.
    bx, by = BADGE_C[0] * u, BADGE_C[1] * u
    ring = (BADGE_R + BADGE_RING) * u
    d.ellipse([bx - ring, by - ring, bx + ring, by + ring], fill=TILE + (255,))
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).ellipse([bx - BADGE_R * u, by - BADGE_R * u, bx + BADGE_R * u, by + BADGE_R * u], fill=255)
    img.paste(gradient(s, MINT_TOP, MINT_BOTTOM), (0, 0), mask)
    font = ImageFont.truetype(str(FONT), int(46 * u))
    d.text((bx, by + 1 * u), "2", font=font, fill=BADGE_TEXT + (255,), anchor="mm")
    return img.resize((size, size), Image.LANCZOS)


def write_svg():
    hexc = lambda c: "#%02x%02x%02x" % c
    pts = lambda p: " ".join(f"{x:.1f},{y:.1f}" for x, y in p)
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <linearGradient id="mint" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{hexc(MINT_TOP)}"/><stop offset="1" stop-color="{hexc(MINT_BOTTOM)}"/></linearGradient>
  </defs>
  <rect width="256" height="256" rx="56" fill="{hexc(TILE)}"/>
  <polygon points="{pts(hexagon(HEX_OUTER))}" fill="url(#mint)"/>
  <polygon points="{pts(hexagon(HEX_INNER))}" fill="{hexc(TILE)}"/>
  <circle cx="{CENTRE[0]}" cy="{CENTRE[1]}" r="{CORE_R}" fill="{hexc(LIGHT)}"/>
  <circle cx="{BADGE_C[0]}" cy="{BADGE_C[1]}" r="{BADGE_R + BADGE_RING}" fill="{hexc(TILE)}"/>
  <circle cx="{BADGE_C[0]}" cy="{BADGE_C[1]}" r="{BADGE_R}" fill="url(#mint)"/>
  <text x="{BADGE_C[0]}" y="{BADGE_C[1]}" dy="0.36em" text-anchor="middle" font-family="Inter, sans-serif" font-weight="600" font-size="46" fill="{hexc(BADGE_TEXT)}">2</text>
</svg>
'''
    (HERE / "klikcore.svg").write_text(svg, encoding="utf-8")


if __name__ == "__main__":
    write_svg()
    render(256).save(HERE / "klikcore-256.png")
    sizes = [16, 24, 32, 48, 64, 128, 256]
    render(256).save(HERE / "klikcore.ico", sizes=[(n, n) for n in sizes])
    print("wrote klikcore.svg, klikcore-256.png, klikcore.ico")
