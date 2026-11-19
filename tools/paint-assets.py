#!/usr/bin/env python3
"""Paints the image layers of the bundled scenes.

Deterministic (fixed seeds), so every PNG can be regenerated and reviewed:

  tools/paint-assets.py            # all scenes
  tools/paint-assets.py papercraft # one scene

Requires Pillow, NumPy and SciPy.
"""
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

SCENES = Path(__file__).resolve().parent.parent / "scenes"



def rng(seed):
    return np.random.default_rng(seed)


def profile(width, base, amplitude, seed, octaves=4, smooth=1.0):
    """A smooth, seamless 1-D height profile in 0..1 (fraction of height)."""
    r = rng(seed)
    x = np.linspace(0.0, 1.0, width, endpoint=False)
    y = np.full(width, base, dtype=np.float64)
    for o in range(octaves):
        freq = (2 ** o) * (2 + r.integers(0, 3))
        amp = amplitude / (1.9 ** o) * smooth ** o
        phase = r.uniform(0, math.tau)
        y += amp * np.sin(math.tau * freq * x + phase)
    return y


def fill_below(width, height, heights, color):
    """RGBA image filled below `heights` (fractions), anti-aliased."""
    yy = np.arange(height)[:, None] + 0.5
    edge = heights[None, :] * height
    alpha = np.clip(yy - edge + 0.5, 0.0, 1.0)
    img = np.zeros((height, width, 4), dtype=np.float64)
    img[..., :3] = np.array(color[:3]) / 255.0
    img[..., 3] = alpha * (color[3] / 255.0 if len(color) > 3 else 1.0)
    return img


def vertical_shade(img, top_color, bottom_color, heights):
    """Recolours an RGBA float image from `top_color` at its edge to `bottom_color`."""
    h, w = img.shape[:2]
    yy = np.arange(h)[:, None] / h
    t = np.clip((yy - heights[None, :]) / np.maximum(1.0 - heights[None, :], 1e-3), 0.0, 1.0)
    for c in range(3):
        img[..., c] = (top_color[c] * (1 - t) + bottom_color[c] * t) / 255.0
    return img


def paper(img, seed, strength=0.06):
    """Multiplies a fibrous paper grain into an RGBA float image."""
    h, w = img.shape[:2]
    r = rng(seed)
    grain = r.normal(0.0, 1.0, (h // 2 + 1, w // 2 + 1))
    grain = np.array(Image.fromarray(((grain * 40) + 128).clip(0, 255).astype(np.uint8)).resize((w, h), Image.BILINEAR), dtype=np.float64) / 255.0
    fibres = r.normal(0.0, 1.0, (h, w // 8 + 1))
    fibres = np.array(Image.fromarray(((fibres * 40) + 128).clip(0, 255).astype(np.uint8)).resize((w, h), Image.BILINEAR), dtype=np.float64) / 255.0
    tex = 1.0 + strength * ((grain - 0.5) * 1.2 + (fibres - 0.5) * 0.8)
    img[..., :3] = np.clip(img[..., :3] * tex[..., None], 0.0, 1.0)
    return img


def to_image(img):
    return Image.fromarray((np.clip(img, 0.0, 1.0) * 255).round().astype(np.uint8), "RGBA")


def from_image(image):
    return np.array(image.convert("RGBA"), dtype=np.float64) / 255.0


def with_shadow(image, offset=(0, 10), blur=14, opacity=0.35, color=(20, 30, 20), grow=0):
    """Returns `image` over a soft drop shadow of its own silhouette, on a
    canvas padded so the shadow is never clipped."""
    pad = blur * 3 + max(abs(offset[0]), abs(offset[1])) + grow
    w, h = image.size
    canvas = Image.new("RGBA", (w + pad * 2, h + pad * 2), (0, 0, 0, 0))
    alpha = Image.new("L", canvas.size, 0)
    alpha.paste(image.getchannel("A"), (pad + offset[0], pad + offset[1]))
    if grow:
        alpha = alpha.filter(ImageFilter.MaxFilter(grow * 2 + 1))
    alpha = alpha.filter(ImageFilter.GaussianBlur(blur)).point(lambda v: int(v * opacity))
    shadow = Image.new("RGBA", canvas.size, (*color, 0))
    shadow.putalpha(alpha)
    canvas = Image.alpha_composite(canvas, shadow)
    canvas.alpha_composite(image, (pad, pad))
    return canvas


def rim(img, color, width=2.0, opacity=0.9):
    """Lightens the top edge of a silhouette (a cut paper or lit ridge)."""
    alpha = img[..., 3]
    h = img.shape[0]
    shifted = np.zeros_like(alpha)
    k = max(1, int(round(width)))
    shifted[k:] = alpha[:-k]
    edge = np.clip(alpha - shifted, 0.0, 1.0) * opacity
    for c in range(3):
        img[..., c] = img[..., c] * (1 - edge) + color[c] / 255.0 * edge
    return img


def save(image, scene, name):
    out = SCENES / scene
    out.mkdir(parents=True, exist_ok=True)
    image.save(out / name, optimize=True)


def circle(draw, cx, cy, r, fill):
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=fill)



def papercraft():
    scene = "papercraft"
    specs = [
        ("hill-1.png", 0.30, 0.10, 11, (178, 216, 200), (156, 200, 182)),
        ("hill-2.png", 0.32, 0.12, 23, (136, 202, 124), (110, 182, 104)),
        ("hill-3.png", 0.36, 0.11, 37, (82, 162, 88), (62, 140, 74)),
        ("hill-4.png", 0.30, 0.08, 51, (46, 122, 66), (32, 98, 54)),
    ]
    for name, base, amp, seed, top, bottom in specs:
        w, h = 2400, 560
        heights = profile(w, base, amp, seed, octaves=3, smooth=0.6)
        img = fill_below(w, h, heights, (*top, 255))
        img = vertical_shade(img, top, bottom, heights)
        img = paper(img, seed, 0.07)
        img = rim(img, (255, 255, 240), 3, 0.55)
        if name == "hill-4.png":
            r = rng(seed)
            im = to_image(img)
            d = ImageDraw.Draw(im)
            for _ in range(110):
                x = r.uniform(0, w)
                top_y = heights[int(x) % w] * h
                y = r.uniform(top_y + 30, h - 20)
                col = [(255, 250, 240), (255, 196, 214), (255, 226, 120), (206, 190, 255)][r.integers(0, 4)]
                s = r.uniform(4, 8)
                for k in range(5):
                    a = k * math.tau / 5
                    circle(d, x + math.cos(a) * s, y + math.sin(a) * s, s * 0.8, col)
                circle(d, x, y, s * 0.6, (255, 214, 90))
            img = from_image(im)
        save(with_shadow(to_image(img), (0, -6), 16, 0.32, (24, 52, 30)), scene, name)

    s = 520
    sun = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(sun)
    circle(d, s / 2, s / 2, s * 0.40, (255, 196, 64, 255))
    circle(d, s / 2, s / 2, s * 0.33, (255, 214, 96, 255))
    sun = to_image(paper(from_image(sun), 5, 0.08))
    save(with_shadow(sun, (0, 8), 14, 0.25, (160, 90, 20)), scene, "sun.png")
    rays = Image.new("RGBA", (900, 900), (0, 0, 0, 0))
    d = ImageDraw.Draw(rays)
    for k in range(12):
        a = k * math.tau / 12
        pts = [
            (450 + math.cos(a - 0.09) * 250, 450 + math.sin(a - 0.09) * 250),
            (450 + math.cos(a) * 430, 450 + math.sin(a) * 430),
            (450 + math.cos(a + 0.09) * 250, 450 + math.sin(a + 0.09) * 250),
        ]
        d.polygon(pts, fill=(255, 226, 140, 255))
    rays = to_image(paper(from_image(rays), 6, 0.08))
    save(with_shadow(rays, (0, 8), 12, 0.2, (160, 90, 20)), scene, "sun-rays.png")

    for name, seed, count in [("cloud-1.png", 3, 7), ("cloud-2.png", 9, 5), ("cloud-3.png", 17, 6)]:
        r = rng(seed)
        w, h = 640, 300
        cloud = Image.new("RGBA", (w, h), (0, 0, 0, 0))
        d = ImageDraw.Draw(cloud)
        for i in range(count):
            t = i / (count - 1)
            cx = 90 + t * (w - 180)
            rr = 70 + 55 * math.sin(math.pi * t) + r.uniform(-12, 12)
            circle(d, cx, h - 80 - rr * 0.55, rr, (255, 255, 255, 255))
        d.rounded_rectangle([60, h - 150, w - 60, h - 40], radius=55, fill=(255, 255, 255, 255))
        arr = from_image(cloud)
        yy = np.linspace(0, 1, h)[:, None]
        belly = np.clip((yy - 0.55) * 2.2, 0, 1) * 0.12
        arr[..., 0] -= belly * 0.6
        arr[..., 1] -= belly * 0.4
        arr[..., 2] -= belly * 0.1
        arr = paper(arr, seed, 0.05)
        save(with_shadow(to_image(arr), (0, 14), 16, 0.22, (40, 70, 110)), scene, name)

    bush = Image.new("RGBA", (420, 200), (0, 0, 0, 0))
    d = ImageDraw.Draw(bush)
    for cx, cy, rr, col in [(110, 130, 80, (70, 150, 80)), (230, 110, 100, (58, 138, 72)), (330, 140, 70, (76, 158, 86))]:
        circle(d, cx, cy, rr, (*col, 255))
    d.rectangle([40, 150, 390, 200], fill=(62, 142, 76, 255))
    arr = paper(from_image(bush), 14, 0.07)
    save(with_shadow(to_image(arr), (4, 8), 10, 0.3, (20, 50, 26)), scene, "bush.png")

    body = Image.new("RGBA", (220, 460), (0, 0, 0, 0))
    d = ImageDraw.Draw(body)
    d.polygon([(70, 120), (150, 120), (185, 460), (35, 460)], fill=(250, 244, 232, 255))
    d.polygon([(60, 124), (110, 60), (160, 124)], fill=(214, 92, 72, 255))
    d.rounded_rectangle([96, 330, 124, 400], radius=12, fill=(150, 104, 70, 255))
    body = to_image(paper(from_image(body), 12, 0.07))
    save(with_shadow(body, (6, 10), 10, 0.3, (20, 50, 26)), scene, "mill.png")
    blades = Image.new("RGBA", (440, 440), (0, 0, 0, 0))
    d = ImageDraw.Draw(blades)
    for k in range(4):
        a = k * math.tau / 4
        cx, cy = 220 + math.cos(a) * 110, 220 + math.sin(a) * 110
        pts = []
        for u, v in [(-100, -22), (100, -30), (100, 30), (-100, 22)]:
            pts.append((cx + math.cos(a) * u - math.sin(a) * v, cy + math.sin(a) * u + math.cos(a) * v))
        d.polygon(pts, fill=(255, 250, 238, 255))
    circle(d, 220, 220, 22, (214, 92, 72, 255))
    blades = to_image(paper(from_image(blades), 13, 0.06))
    save(with_shadow(blades, (6, 10), 10, 0.25, (20, 50, 26)), scene, "blades.png")



def signal():
    scene = "signal"
    cell = Image.new("RGBA", (96, 96), (0, 0, 0, 0))
    d = ImageDraw.Draw(cell)
    d.line([(0, 0), (95, 0)], fill=(80, 255, 170, 46), width=1)
    d.line([(0, 0), (0, 95)], fill=(80, 255, 170, 46), width=1)
    for k in range(1, 4):
        d.point([(k * 24, 48), (48, k * 24)], fill=(80, 255, 170, 60))
    save(cell, scene, "grid.png")
    lines = Image.new("RGBA", (4, 4), (0, 0, 0, 0))
    ImageDraw.Draw(lines).line([(0, 0), (3, 0)], fill=(0, 0, 0, 90))
    save(lines, scene, "scanlines.png")



def fbm(h, w, seed, octaves=5, base=4, wrap_x=False, persistence=0.5):
    """Fractal value noise in 0..1; `wrap_x` makes it tile horizontally."""
    r = rng(seed)
    out = np.zeros((h, w))
    amp, total = 1.0, 0.0
    for o in range(octaves):
        gx, gy = base * 2 ** o, max(2, int(base * 2 ** o * h / w))
        grid = r.random((gy + 3, gx + 3))
        if wrap_x:
            grid[:, gx:gx + 3] = grid[:, 0:3]
        big = np.array(Image.fromarray((grid * 255).astype(np.uint8)).resize(
            (int(w * (gx + 3) / gx), int(h * (gy + 3) / gy)), Image.BICUBIC), dtype=np.float64) / 255
        ox = int(w * 1 / gx) if wrap_x else 0
        out += amp * big[int(h / gy):int(h / gy) + h, ox:ox + w]
        total += amp
        amp *= persistence
    return np.clip(out / total, 0, 1)


def washi(size, seed):
    """Cream paper fibres as a multiply texture (white = no change)."""
    n = fbm(size, size, seed, 5, 6, wrap_x=True)
    r = rng(seed + 1)
    fib = np.ones((size, size))
    im = Image.new("L", (size, size), 255)
    d = ImageDraw.Draw(im)
    for _ in range(120):
        x, y = r.uniform(0, size), r.uniform(0, size)
        a = r.uniform(0, math.tau)
        l = r.uniform(10, 40)
        d.line([(x, y), (x + math.cos(a) * l, y + math.sin(a) * l)], fill=int(r.uniform(236, 248)), width=1)
    fib = np.array(im.filter(ImageFilter.GaussianBlur(0.6)), dtype=np.float64) / 255
    v = np.clip(0.93 + (n - 0.5) * 0.12, 0, 1) * fib
    img = np.zeros((size, size, 4))
    img[..., 0] = v
    img[..., 1] = v * 0.985
    img[..., 2] = v * 0.95
    img[..., 3] = 1.0
    return img



INDIGO = np.array([22, 44, 84]) / 255
PRUSSIAN = np.array([28, 70, 120]) / 255
SEA_LIGHT = np.array([96, 142, 178]) / 255
FOAM = np.array([246, 240, 222]) / 255
KEY = np.array([18, 30, 54]) / 255


def wave_band(w, h, seed, crests, amp, base, lines=8):
    """Seamless woodcut swells with broken highlights instead of pointed crests."""
    r = rng(seed)
    x = np.arange(w) / w
    surface = np.zeros(w)
    for i in range(crests):
        peak = (i + r.uniform(0.12, 0.84)) / crests
        distance = ((x - peak + 0.5) % 1 - 0.5) * crests
        width = np.where(distance < 0, 0.62, 0.29) * r.uniform(0.78, 1.15)
        surface += r.uniform(0.56, 1.0) * np.exp(-(distance / width) ** 2)
    top = base - amp * surface + amp * 0.10 * np.cos(math.tau * x)
    depth = np.arange(h)[:, None] / h - top
    inside = np.clip(depth * h + 0.5, 0, 1)
    ink = fbm(h, w, seed + 13, 4, 9, wrap_x=True)
    t = np.clip(depth / 0.29, 0, 1)
    col = SEA_LIGHT * (1 - t[..., None]) ** 1.6 + PRUSSIAN * (1 - (1 - t[..., None]) ** 1.6)
    col = col * (1 - 0.40 * t[..., None]) + INDIGO * 0.40 * t[..., None]
    spacing = 0.013
    k = (depth + 0.002 * (ink - 0.5)) / spacing
    line = np.clip(1 - np.abs(k - np.round(k)) * spacing * h / 1.9, 0, 1)
    line *= (k > 0.6) * np.clip(1 - k / lines, 0, 1)
    col = col * (1 - 0.55 * line[..., None]) + INDIGO * 0.55 * line[..., None]
    highlight = np.clip((0.003 + 0.005 * surface - depth) * h, 0, 1) * inside
    highlight *= np.clip((ink - 0.38) * 4, 0.04, 0.72)
    col = col * (1 - highlight[..., None]) + FOAM * highlight[..., None]
    key = np.clip(1 - np.abs(depth * h + 0.3) / 1.7, 0, 1)
    col = col * (1 - key[..., None]) + KEY * key[..., None]
    img = np.zeros((h, w, 4))
    img[..., :3] = col
    img[..., 3] = np.maximum(inside, key)
    return to_image(img)


def catmull(points, steps=24):
    pts = [points[0]] + points + [points[-1]]
    out = []
    for i in range(1, len(pts) - 2):
        p0, p1, p2, p3 = (np.array(pts[j]) for j in (i - 1, i, i + 1, i + 2))
        for k in range(steps):
            t = k / steps
            out.append(tuple(0.5 * (2 * p1 + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t + (-p0 + 3 * p1 - 3 * p2 + p3) * t ** 3)))
    out.append(points[-1])
    return out


def great_wave(seed=5):
    """The hero wave: a tall wall curling over into a clawed lip."""
    from scipy import ndimage
    W, H = 1800, 1300
    r = rng(seed)
    outline = [(0.0, 1.0), (0.0, 0.66), (0.05, 0.5), (0.13, 0.34), (0.24, 0.2), (0.37, 0.1), (0.5, 0.05),
               (0.62, 0.06), (0.73, 0.12), (0.81, 0.21), (0.855, 0.31), (0.84, 0.36), (0.8, 0.33), (0.75, 0.27),
               (0.68, 0.23), (0.61, 0.25), (0.575, 0.32), (0.575, 0.43), (0.61, 0.55), (0.69, 0.66), (0.8, 0.74),
               (0.9, 0.8), (1.0, 0.9), (1.0, 1.0)]
    poly = [(x * W, y * H) for x, y in catmull(outline, 30)]
    mask_im = Image.new("L", (W, H), 0)
    ImageDraw.Draw(mask_im).polygon(poly, fill=255)
    mask = np.array(mask_im, dtype=np.float64) / 255
    inside = mask > 0.5
    dist = ndimage.distance_transform_edt(inside)
    yy, xx = np.mgrid[0:H, 0:W]
    ny = yy / H
    t = np.clip(dist / 260, 0, 1)
    col = SEA_LIGHT * (1 - t[..., None]) ** 1.4 + PRUSSIAN * (1 - (1 - t[..., None]) ** 1.4)
    col = col * (1 - 0.45 * np.clip(ny, 0, 1)[..., None]) + INDIGO * 0.45 * np.clip(ny, 0, 1)[..., None]
    spacing = 17.0
    k = dist / spacing
    line = np.clip(1 - np.abs(k - np.round(k)) * spacing / 1.4, 0, 1) * (k > 1.2) * np.clip(1 - k / 14, 0, 1)
    col = col * (1 - 0.6 * line[..., None]) + INDIGO * 0.6 * line[..., None]
    vein_k = dist / (spacing * 2)
    vein = np.clip(1 - np.abs(vein_k - np.round(vein_k)) * spacing * 2 / 2.2, 0, 1) * (vein_k > 1.4)
    vein *= np.clip((0.5 - ny) / 0.3, 0, 1) * (fbm(H, W, seed + 40, 4, 6) > 0.45) * (dist < 110)
    col = col * (1 - 0.8 * vein[..., None]) + FOAM * 0.8 * vein[..., None]
    foam_w = 30 * np.clip((0.4 - ny) / 0.3, 0, 1) ** 0.8
    foam = (dist < foam_w) & inside
    col[foam] = FOAM
    key = (dist > 0) & (dist < 2.2)
    col[key] = KEY
    img = np.zeros((H, W, 4))
    img[..., :3] = col
    img[..., 3] = mask
    im = to_image(img)
    d = ImageDraw.Draw(im)
    foam_c = tuple((FOAM * 255).astype(int)) + (255,)
    lip = catmull([(0.41, 0.081), (0.5, 0.05), (0.62, 0.06), (0.73, 0.12),
                   (0.81, 0.21), (0.855, 0.31), (0.84, 0.36)], 24)
    scallops = [(0.10, 18, 15), (0.14, 27, 16), (0.20, 22, 13),
                (0.32, 30, 16), (0.38, 40, 23), (0.43, 25, 13),
                (0.53, 46, 24), (0.57, 36, 18), (0.65, 56, 26),
                (0.69, 48, 22), (0.73, 38, 19), (0.81, 63, 24),
                (0.84, 51, 25), (0.89, 66, 23), (0.92, 52, 20), (0.96, 34, 12)]
    for progress, crest, width in scallops:
        i = round(progress * (len(lip) - 1))
        (ax, ay), (bx, by) = lip[i - 1], lip[i + 1]
        tx, ty = (bx - ax) * W, (by - ay) * H
        length = math.hypot(tx, ty)
        tx, ty = tx / length, ty / length
        nx, ny_ = ty, -tx
        x0, y0 = lip[i][0] * W, lip[i][1] * H
        # Each scallop starts inside the wave's foam and curls in the direction of travel.
        contour = [(-1.13, -7), (-0.88, crest * 0.27), (-0.42, crest * 0.62),
                   (0.21, crest * 0.93), (1.1, crest), (1.7, crest * 0.84),
                   (1.04, crest * 0.71), (0.68, crest * 0.43), (0.63, -9)]
        path = [(x0 + tx * u * width + nx * v, y0 + ty * u * width + ny_ * v)
                for u, v in contour]
        d.polygon(catmull(path, 12), fill=foam_c)
        curl = catmull([(x0 + tx * u * width + nx * v, y0 + ty * u * width + ny_ * v)
                        for u, v in [(-0.81, crest * 0.24), (-0.45, crest * 0.54),
                                     (0.28, crest * 0.83), (1.15, crest * 0.96)]], 10)
        d.line(curl, fill=(120, 143, 160, 255), width=2)
        for _ in range(int(3 + 4 * progress)):
            along = r.uniform(-16, 43)
            lift = crest + r.uniform(5, 67)
            circle(d, x0 + tx * along + nx * lift, y0 + ty * along + ny_ * lift,
                   r.uniform(1.45, 4.0), foam_c)
    return im

def kanagawa():
    scene = "kanagawa"
    w = 2400
    specs = [
        ("sea-1.png", 11, 7, 0.087, 0.33, 5),
        ("sea-2.png", 23, 6, 0.117, 0.33, 7),
        ("sea-3.png", 37, 5, 0.18, 0.36, 9),
        ("sea-4.png", 51, 4, 0.22, 0.40, 11),
    ]
    for name, seed, crests, amp, base, lines in specs:
        save(wave_band(w, 700, seed, crests, amp, base, lines), scene, name)

    fw, fh = 1400, 560
    xs = np.linspace(-1, 1, fw)
    slope = 1 - np.abs(xs) ** 0.62
    top = 1 - slope * 0.92
    top = np.maximum(top, 0.08 + 0.02 * np.sin(xs * 40))
    yy = np.arange(fh)[:, None] / fh
    inside = np.clip((yy - top[None, :]) * fh + 0.5, 0, 1)
    snowline = 0.42 + 0.05 * np.sin(xs * 23) + 0.035 * np.sin(xs * 61 + 1) + 0.02 * np.sin(xs * 131)
    snow = np.clip((snowline[None, :] - yy) * fh, 0, 1)
    t = np.clip((yy - 0.1) / 0.9, 0, 1)
    body = np.array([70, 96, 130]) / 255 * (1 - t[..., None]) + np.array([118, 140, 160]) / 255 * t[..., None]
    col = body * (1 - snow[..., None]) + FOAM * snow[..., None]
    key = np.clip(1 - np.abs((yy - top[None, :]) * fh - 0.5) / 1.8, 0, 1)
    col = col * (1 - key[..., None]) + KEY * key[..., None]
    img = np.zeros((fh, fw, 4))
    img[..., :3] = col
    img[..., 3] = np.maximum(inside, key) * np.clip((1 - yy) * 6, 0, 1)
    save(to_image(img), scene, "fuji.png")

    for name, seed, length, puffs in [("cloud-1.png", 3, 1100, 7), ("cloud-2.png", 5, 760, 5)]:
        r = rng(seed)
        ch = 150
        S2 = 2
        mask = Image.new("L", (length * S2, ch * S2), 0)
        d = ImageDraw.Draw(mask)
        d.rounded_rectangle([0, 70 * S2, length * S2, 112 * S2], radius=20 * S2, fill=255)
        xs = np.linspace(length * 0.12, length * 0.88, puffs)
        for x in xs:
            rx = r.uniform(60, 95)
            ry = r.uniform(30, 48)
            x = (x + r.uniform(-25, 25))
            d.ellipse([(x - rx) * S2, (92 - ry * 1.6) * S2, (x + rx) * S2, (92 + ry * 0.4) * S2], fill=255)
        mask = mask.resize((length, ch), Image.LANCZOS)
        a_ = np.array(mask, dtype=np.float64) / 255
        xx = np.linspace(0, 1, length)[None, :]
        ends = np.clip(np.minimum(xx, 1 - xx) / 0.12, 0, 1) ** 0.8
        yy = np.arange(ch)[:, None] / ch
        img = np.zeros((ch, length, 4))
        top_c = np.array([246, 236, 214]) / 255
        low_c = np.array([214, 196, 166]) / 255
        t = np.clip((yy - 0.25) / 0.55, 0, 1)
        img[..., :3] = top_c * (1 - t[..., None]) + low_c * t[..., None]
        img[..., 3] = a_ * ends * 0.92
        save(to_image(img), scene, name)

    boat = Image.new("RGBA", (680, 500))
    d = ImageDraw.Draw(boat)
    ink = (28, 47, 65, 255)
    frame = [(46, 412), (162, 424), (444, 423), (599, 396), (660, 368),
             (622, 420), (552, 452), (172, 460), (86, 445)]
    frame = catmull([*frame, frame[0]])
    d.polygon(frame, fill=(103, 77, 61, 255))
    d.line(frame, fill=ink, width=7)
    d.line(catmull([(62, 418), (180, 441), (442, 439), (580, 415), (641, 385)]),
           fill=(210, 164, 107, 255), width=6)
    d.line([(337, 433), (398, 58)], fill=ink, width=7)
    d.line([(395, 84), (639, 393)], fill=(58, 65, 63, 255), width=4)
    sail = [(360, 79), (301, 144), (213, 247), (95, 382), (217, 361),
            (299, 376), (353, 391), (361, 252), (370, 142)]
    sail = catmull([*sail, sail[0]])
    d.polygon(sail, fill=(222, 202, 163, 255))
    for i in range(5):
        t = i / 4
        rib = catmull([(356 + t * 12, 93), (283 + t * 78, 213), (204 + t * 132, 365 + t * 20)])
        d.line(rib, fill=(170, 143, 110, 255), width=4)
    d.line(sail, fill=ink, width=5)
    d.line([(351, 392), (374, 438), (439, 427)], fill=ink, width=4)
    d.line(catmull([(14, 460), (109, 474), (239, 475), (347, 467)]),
           fill=(164, 188, 199, 175), width=4)
    d.line(catmull([(26, 478), (105, 485), (196, 482)]),
           fill=(136, 172, 190, 145), width=3)
    save(to_image(paper(from_image(boat), 69, 0.065)), scene, "boat.png")

    s_ = 520
    yy, xx = np.mgrid[0:s_, 0:s_] + 0.5
    n = fbm(s_, s_, 91, 5, 5)
    q = np.maximum(np.abs(xx - s_ / 2), np.abs(yy - s_ / 2)) - s_ * 0.42
    edge = q + (n - 0.5) * 26
    body = np.clip(-edge, 0, 1)
    inner = np.abs(q + s_ * 0.05) < s_ * 0.012
    wear = (fbm(s_, s_, 92, 6, 8) > 0.72) * 0.55
    alpha = body * (1 - wear) * (1 - inner * 0.85)
    img = np.zeros((s_, s_, 4))
    img[..., 0], img[..., 1], img[..., 2] = 196 / 255, 58 / 255, 40 / 255
    img[..., 3] = alpha
    save(to_image(img), scene, "seal.png")

    cw, ch = 150, 620
    im = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rectangle([4, 4, cw - 4, ch - 4], fill=(236, 214, 158, 255), outline=(40, 36, 44, 255), width=5)
    d.rectangle([16, 16, cw - 16, ch - 16], outline=(40, 36, 44, 255), width=2)
    save(im, scene, "cartouche.png")

    save(great_wave(), scene, "great-wave.png")
    save(to_image(washi(512, 7)), scene, "washi.png")


def tide():
    grain("tide", "grain.png", 0.05, seed=11)



PINK = np.array([255, 72, 176]) / 255
BLUE = np.array([0, 120, 191]) / 255


def halftone(tone, cell, angle):
    """Coverage of a rotated dot screen for a 0..1 tone field."""
    h, w = tone.shape
    yy, xx = np.mgrid[0:h, 0:w] + 0.5
    ca, sa = math.cos(angle), math.sin(angle)
    u = (xx * ca + yy * sa) / cell
    v = (-xx * sa + yy * ca) / cell
    du, dv = u - np.round(u), v - np.round(v)
    d = np.hypot(du, dv) * cell
    radius = np.sqrt(np.clip(tone, 0, 1)) * cell * 0.62
    return np.clip(radius - d + 0.5, 0, 1)


def ink(coverage, color, seed, dropout=0.08):
    """An ink layer for multiply blending: white where no ink, with
    speckled dropout like a drum print."""
    h, w = coverage.shape
    speck = (rng(seed).random((h, w)) < dropout) * 0.8 + (fbm(h, w, seed, 4, 8) - 0.5) * 0.3
    cov = np.clip(coverage * (1 - speck), 0, 1)
    img = np.ones((h, w, 4))
    img[..., :3] = 1 - cov[..., None] * (1 - color)
    img[..., 3] = cov
    return to_image(img)


def skyline_mask(w, h, seed, count, min_h, max_h, windows=True):
    r = rng(seed)
    body = Image.new("L", (w, h), 0)
    holes = Image.new("L", (w, h), 0)
    d, hd = ImageDraw.Draw(body), ImageDraw.Draw(holes)
    x = -20
    while x < w + 20:
        bw = r.uniform(70, 190)
        bh = r.uniform(min_h, max_h) * h
        top = h - bh
        d.rectangle([x, top, x + bw, h], fill=255)
        kind = r.integers(0, 5)
        if kind == 0:
            d.rectangle([x + bw * 0.35, top - 60, x + bw * 0.65, top], fill=255)
            d.line([(x + bw / 2, top - 60), (x + bw / 2, top - 130)], fill=255, width=4)
        elif kind == 1:
            d.ellipse([x + bw * 0.2, top - 70, x + bw * 0.55, top - 20], fill=255)
            d.rectangle([x + bw * 0.25, top - 30, x + bw * 0.5, top], fill=255)
        elif kind == 2:
            d.polygon([(x, top), (x + bw / 2, top - bw * 0.4), (x + bw, top)], fill=255)
        if windows:
            for wy in np.arange(top + 24, h - 20, 34):
                for wx in np.arange(x + 14, x + bw - 18, 26):
                    if r.random() < 0.55:
                        hd.rectangle([wx, wy, wx + 12, wy + 16], fill=255)
        x += bw + r.uniform(-24, -4)
    return np.array(body, dtype=np.float64) / 255, np.array(holes, dtype=np.float64) / 255


def riso():
    scene = "riso"
    W, H = 2400, 1000
    body, _ = skyline_mask(W, H, 3, 30, 0.35, 0.8, windows=False)
    tone = body * 0.42
    save(ink(halftone(tone, 9, math.radians(15)), BLUE, 3), scene, "city-far.png")
    body, holes = skyline_mask(W, H, 7, 18, 0.25, 0.62)
    shade = body * 0.9
    cov = np.clip(shade - holes, 0, 1)
    save(ink(cov, BLUE, 7), scene, "city-near.png")
    lit = holes * (rng(9).random(holes.shape) > 0.0)
    r_ = rng(10)
    lit_im = Image.fromarray((holes * 255).astype(np.uint8))
    lit_mask = np.array(lit_im, dtype=np.float64) / 255 * (fbm(H, W, 11, 3, 40) > 0.5)
    save(ink(lit_mask, PINK, 11, 0.02), scene, "windows.png")
    S = 900
    yy, xx = np.mgrid[0:S, 0:S] + 0.5
    rad = np.hypot(xx - S / 2, yy - S / 2) / (S / 2)
    tone = np.clip((1 - rad) * 30, 0, 1) * (0.35 + 0.65 * (1 - yy / S))
    save(ink(halftone(tone, 11, math.radians(75)), PINK, 13), scene, "sun.png")
    for name, seed in [("cloud-1.png", 21), ("cloud-2.png", 22)]:
        cw, ch = 1000, 320
        m = Image.new("L", (cw, ch), 0)
        d = ImageDraw.Draw(m)
        rr = rng(seed)
        for i in range(7):
            t = i / 6
            cx_ = 180 + t * (cw - 360)
            rad_ = 55 + 45 * math.sin(math.pi * t) + rr.uniform(-10, 10)
            d.ellipse([cx_ - rad_, ch * 0.62 - rad_ * 1.1, cx_ + rad_, ch * 0.62 + rad_ * 0.4], fill=255)
        d.rounded_rectangle([120, ch * 0.5, cw - 120, ch * 0.72], radius=40, fill=255)
        tone = np.array(m.filter(ImageFilter.GaussianBlur(14)), dtype=np.float64) / 255 * 0.6
        save(ink(halftone(tone, 8, math.radians(75)), PINK, seed), scene, name)
    GW, GH = 2400, 160
    g = np.zeros((GH, GW))
    g[60:] = 1.0
    g[70:74] = 0.0
    g[96:100] = 0.0
    for x in range(0, GW, 40):
        g[74:96, x:x + 10] = 0.0
    save(ink(g, BLUE, 31), scene, "ground.png")
    TW, TH = 700, 260
    t = Image.new("L", (TW, TH), 0)
    tb = Image.new("L", (TW, TH), 0)
    d, db = ImageDraw.Draw(t), ImageDraw.Draw(tb)
    d.rounded_rectangle([20, 60, TW - 20, 220], radius=40, fill=255)
    d.line([(TW / 2, 60), (TW / 2 - 60, 0), (TW / 2 + 40, 0)], fill=255, width=6)
    for i in range(6):
        db.rounded_rectangle([60 + i * 100, 90, 130 + i * 100, 150], radius=10, fill=255)
    for wx in (140, 280, 420, 560):
        db.ellipse([wx - 26, 200, wx + 26, 252], fill=255)
    body = np.array(t, dtype=np.float64) / 255
    blue_ = np.array(tb, dtype=np.float64) / 255
    pink_im = ink(np.clip(body - blue_ * 0.6, 0, 1), PINK, 41, 0.03)
    blue_im = ink(blue_, BLUE, 42, 0.03)
    tram = Image.new("RGBA", (TW, TH), (255, 255, 255, 0))
    tram.alpha_composite(pink_im)
    a = np.array(tram, dtype=np.float64) / 255
    b = np.array(blue_im, dtype=np.float64) / 255
    out = a.copy()
    out[..., :3] = a[..., :3] * (1 - b[..., 3:4]) + (a[..., :3] * b[..., :3]) * b[..., 3:4]
    out[..., 3] = np.maximum(a[..., 3], b[..., 3])
    out[..., :3] = np.where(a[..., 3:4] > 0, out[..., :3], b[..., :3])
    save(to_image(out), scene, "tram.png")
    P = 512
    n = fbm(P, P, 51, 5, 6, wrap_x=True)
    speck = rng(52).random((P, P)) < 0.004
    v = 0.965 + (n - 0.5) * 0.05 - speck * 0.25
    img = np.ones((P, P, 4))
    img[..., 0] = v
    img[..., 1] = v * 0.985
    img[..., 2] = v * 0.94
    save(to_image(img), scene, "paper.png")



def grain(scene, name, strength, size=256, seed=1):
    r = rng(seed)
    noise = r.normal(0.0, 1.0, (size, size))
    img = np.zeros((size, size, 4))
    img[..., :3] = (noise[..., None] > 0).astype(float)
    img[..., 3] = np.clip(np.abs(noise) * strength, 0, 1)
    save(to_image(img), scene, name)


def bokeh(scene):
    s = 128
    yy, xx = np.mgrid[0:s, 0:s] + 0.5
    r = np.hypot(xx - s / 2, yy - s / 2) / (s / 2)
    a = np.clip((1 - r) * 18, 0, 1) * (0.55 + 0.45 * np.clip(r, 0, 1) ** 3)
    img = np.ones((s, s, 4))
    img[..., 3] = a
    save(to_image(img), scene, "bokeh.png")


def bloom():
    grain("bloom", "grain.png", 0.10, seed=3)
    bokeh("bloom")


def bauhaus():
    grain("bauhaus", "grain.png", 0.07, seed=5)
    s = 64
    yy, xx = np.mgrid[0:s, 0:s] + 0.5
    img = np.zeros((s, s, 4))
    img[..., :3] = 28 / 255
    img[..., 3] = np.clip(s * 0.16 - np.hypot(xx - s / 2, yy - s / 2) + 0.5, 0, 1)
    save(to_image(img), "bauhaus", "dot.png")


def still():
    grain("still", "grain.png", 0.05, seed=7)


PAINTERS = {
    "papercraft": papercraft,
    "signal": signal,
    "bloom": bloom,
    "bauhaus": bauhaus,
    "still": still,
    "kanagawa": kanagawa,
    "tide": tide,
    "riso": riso,
}

if __name__ == "__main__":
    for name in sys.argv[1:] or PAINTERS:
        PAINTERS[name]()
        print("painted", name)
