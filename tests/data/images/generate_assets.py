#!/usr/bin/env python3
"""
Generate the sample images used by image-test.html.

Run from anywhere:  python3 generate_assets.py
Outputs into ./assets next to this script.

Each raster image is 400x300 and carries a big format label so you can tell at a
glance which decoder produced the pixels on screen. Special cases are included:
  - logo.png        : RGBA with real transparency (alpha compositing test)
  - sprite.gif      : palette image with a transparent colour-key
  - tile.png        : 64x64 seamless tile for background-repeat
  - wide.jpg        : 800x200 gradient for background-size: cover/contain
  - vector.svg / tile.svg are written by image-test generation directly (text file)
"""

import os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "assets")
os.makedirs(OUT, exist_ok=True)

FONT_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


def font(size):
    try:
        return ImageFont.truetype(FONT_PATH, size)
    except OSError:
        return ImageFont.load_default()


def centered(draw, box, text, fnt, fill):
    x0, y0, x1, y1 = box
    l, t, r, b = draw.textbbox((0, 0), text, font=fnt)
    tw, th = r - l, b - t
    draw.text((x0 + (x1 - x0 - tw) / 2 - l, y0 + (y1 - y0 - th) / 2 - t),
              text, font=fnt, fill=fill)


def gradient(size, c0, c1):
    """Vertical gradient image (RGB)."""
    w, h = size
    img = Image.new("RGB", size)
    px = img.load()
    for y in range(h):
        t = y / max(h - 1, 1)
        col = tuple(int(c0[i] + (c1[i] - c0[i]) * t) for i in range(3))
        for x in range(w):
            px[x, y] = col
    return img


def labelled(label, c0, c1, sub=""):
    img = gradient((400, 300), c0, c1).convert("RGBA")
    d = ImageDraw.Draw(img)
    # diagonal stripe so scaling/aspect issues are obvious
    d.line([(0, 300), (400, 0)], fill=(255, 255, 255, 90), width=6)
    d.rectangle([4, 4, 395, 295], outline=(255, 255, 255, 200), width=3)
    centered(d, (0, 70, 400, 200), label, font(96), (255, 255, 255, 255))
    if sub:
        centered(d, (0, 200, 400, 270), sub, font(26), (255, 255, 255, 220))
    return img


# ── JPEG : lossy, no alpha ────────────────────────────────────────────────────
labelled("JPEG", (200, 60, 40), (90, 10, 10), "lossy / no alpha") \
    .convert("RGB").save(os.path.join(OUT, "photo.jpg"), quality=85)

# ── PNG : RGBA with real transparency ─────────────────────────────────────────
png = labelled("PNG", (30, 120, 200), (10, 40, 90), "RGBA transparency")
# punch a transparent circle so the page background shows through
mask = Image.new("L", png.size, 0)
ImageDraw.Draw(mask).ellipse([300, 200, 380, 280], fill=255)
alpha = png.getchannel("A")
alpha.paste(0, (0, 0), mask)
png.putalpha(alpha)
png.save(os.path.join(OUT, "logo.png"))

# ── GIF : palette + transparent colour-key ────────────────────────────────────
gif = labelled("GIF", (40, 160, 70), (10, 60, 25), "256-colour palette")
gif_rgb = gif.convert("RGB")
p = gif_rgb.convert("P", palette=Image.ADAPTIVE, colors=255)
# reserve index 255 as transparent and stamp a transparent corner
pd = ImageDraw.Draw(p)
pd.rectangle([320, 220, 396, 296], fill=255)
p.save(os.path.join(OUT, "sprite.gif"), transparency=255)

# ── WebP : lossy ──────────────────────────────────────────────────────────────
labelled("WebP", (150, 90, 200), (50, 20, 80), "lossy") \
    .convert("RGB").save(os.path.join(OUT, "picture.webp"), quality=85)

# ── tile.png : 64x64 seamless tile for background-repeat ──────────────────────
tile = Image.new("RGBA", (64, 64), (245, 246, 248, 255))
td = ImageDraw.Draw(tile)
td.rectangle([0, 0, 31, 31], fill=(220, 228, 240, 255))
td.rectangle([32, 32, 63, 63], fill=(220, 228, 240, 255))
td.ellipse([22, 22, 42, 42], fill=(120, 160, 220, 255))
tile.save(os.path.join(OUT, "tile.png"))

# ── wide.jpg : 800x200 gradient for background-size cover/contain ──────────────
wide = gradient((800, 200), (255, 170, 60), (200, 40, 120))
wd = ImageDraw.Draw(wide)
centered(wd, (0, 0, 800, 200), "background  cover / contain", font(48), (255, 255, 255))
wide.save(os.path.join(OUT, "wide.jpg"), quality=85)

print("wrote:", ", ".join(sorted(os.listdir(OUT))))
