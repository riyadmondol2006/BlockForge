"""Builds the BlockForge logo SVGs and exports every PNG/ICO/ICNS the launcher uses.

The logo is a glowing ember block resting on an anvil, on a green rounded tile.

    pip install --user cairosvg fonttools uharfbuzz pillow
    python3 docs/logo/export.py            # run from the repository root

See docs/logo/README.md for what gets written where.
"""
import io
import math
import struct
from pathlib import Path

import cairosvg
import uharfbuzz as hb
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
LOGO = ROOT / "docs/logo"
IMAGES = ROOT / "src/main/resources/assets/image"
FONTS = ROOT / "src/main/resources/assets/font"
LINUX_ICONS = [ROOT / "packaging/linux/_common", ROOT / "packaging/aur/atlauncher",
               ROOT / "packaging/aur/atlauncher-bin"]

# colours (greens follow gui/modern/Palette.java)
GREEN_TOP, GREEN_BOTTOM = "#36B964", "#0E5A2B"
ANVIL = "#F4F9F1"
EMBER_TOP_A, EMBER_TOP_B = "#FFE28C", "#FFC456"
EMBER_LEFT, EMBER_RIGHT = "#FF9F40", "#EA6427"
SPARK = "#FFF1C2"
THEMES = {
    # name: (background, "Block", "Forge", subtitle)
    "dark": ("#10140F", "#E7ECE4", "#5FD97A", "#9BA89C"),
    "light": ("#F7FBF1", "#141A15", "#1B8A3C", "#586558"),
}


# ---------------------------------------------------------------- geometry helpers

def f(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def rpoly(pts, r):
    """Closed polygon path with rounded (quadratic) corners; r is one radius or one per corner."""
    n = len(pts)
    radii = r if isinstance(r, (list, tuple)) else [r] * n
    out = []
    for i in range(n):
        p0, p1, p2 = pts[i - 1], pts[i], pts[(i + 1) % n]
        d0, d2 = math.dist(p0, p1), math.dist(p1, p2)
        k0, k2 = min(radii[i], d0 / 2) / d0, min(radii[i], d2 / 2) / d2
        a = (p1[0] + (p0[0] - p1[0]) * k0, p1[1] + (p0[1] - p1[1]) * k0)
        b = (p1[0] + (p2[0] - p1[0]) * k2, p1[1] + (p2[1] - p1[1]) * k2)
        out.append(("L" if i else "M") + f"{f(a[0])} {f(a[1])}")
        out.append(f"Q{f(p1[0])} {f(p1[1])} {f(b[0])} {f(b[1])}")
    return "".join(out) + "Z"


def inset(poly, d):
    """Shrink a convex polygon by moving every edge inwards by d."""
    cx = sum(p[0] for p in poly) / len(poly)
    cy = sum(p[1] for p in poly) / len(poly)

    def shifted(a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        length = math.hypot(dx, dy)
        nx, ny = -dy / length, dx / length
        if (cx - (a[0] + b[0]) / 2) * nx + (cy - (a[1] + b[1]) / 2) * ny < 0:
            nx, ny = -nx, -ny
        return (a[0] + nx * d, a[1] + ny * d), (b[0] + nx * d, b[1] + ny * d)

    out = []
    n = len(poly)
    for i in range(n):
        (x1, y1), (x2, y2) = shifted(poly[i - 1], poly[i])
        (x3, y3), (x4, y4) = shifted(poly[i], poly[(i + 1) % n])
        den = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4)
        t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / den
        out.append((x1 + t * (x2 - x1), y1 + t * (y2 - y1)))
    return out


def star4(cx, cy, r, inner=0.3):
    """Four point sparkle with concave sides."""
    pts = [(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)]
    d = f"M{f(pts[0][0])} {f(pts[0][1])}"
    for i in range(4):
        p, q = pts[i], pts[(i + 1) % 4]
        c1 = (cx + (p[0] - cx) * inner, cy + (p[1] - cy) * inner)
        c2 = (cx + (q[0] - cx) * inner, cy + (q[1] - cy) * inner)
        d += f"C{f(c1[0])} {f(c1[1])} {f(c2[0])} {f(c2[1])} {f(q[0])} {f(q[1])}"
    return d + "Z"


# ---------------------------------------------------------------- the mark

def anvil_path(T, k=1.0, cx=256):
    """Anvil seen from the side. T is the y of the working face, k scales the shape around (cx, T)."""
    def p(x, y):
        return f"{f(cx + (x - 256) * k)} {f(T + y * k)}"

    return ("M" + p(84, 8) + "Q" + p(80, 4) + " " + p(88, 3) +          # horn tip
            "L" + p(176, 0) +                                          # horn top, flush with the face
            "L" + p(420, 0) + "Q" + p(432, 0) + " " + p(432, 12) +
            "L" + p(432, 42) + "Q" + p(432, 54) + " " + p(420, 54) +
            "L" + p(384, 54) +
            "C" + p(362, 54) + " " + p(352, 66) + " " + p(352, 84) +   # waist
            "C" + p(352, 98) + " " + p(370, 106) + " " + p(392, 110) +  # foot
            "Q" + p(404, 112) + " " + p(404, 124) + "Q" + p(404, 132) + " " + p(396, 132) +
            "L" + p(216, 132) +
            "Q" + p(208, 132) + " " + p(208, 124) + "Q" + p(208, 112) + " " + p(220, 110) +
            "C" + p(242, 106) + " " + p(260, 98) + " " + p(260, 84) +
            "C" + p(260, 66) + " " + p(250, 54) + " " + p(228, 54) +
            "C" + p(170, 54) + " " + p(118, 30) + " " + p(84, 8) + "Z")  # horn underside


def cube(cx, T, s, gap, r):
    """Isometric block whose two lower side corners sit on the line y = T (the anvil face hides the rest)."""
    c = math.cos(math.radians(30)) * s
    cy = T - s / 2 + gap  # sink it a little so the inset side faces still reach the anvil face
    top, ur, lr = (cx, cy - s), (cx + c, cy - s / 2), (cx + c, cy + s / 2)
    bot, ll, ul, mid = (cx, cy + s), (cx - c, cy + s / 2), (cx - c, cy - s / 2), (cx, cy)
    faces = [[top, ur, mid, ul], [ul, mid, bot, ll], [mid, ur, lr, bot]]
    radii = [r, [r, r, 0, 0], [r, r, 0, 0]]  # corners hidden behind the anvil stay sharp
    if gap:
        faces = [inset(q, gap / 2) for q in faces]
    return "".join(f'<path d="{rpoly(q, rr)}" fill="{col}"/>'
                   for q, rr, col in zip(faces, radii, ("url(#bf-ember)", EMBER_LEFT, EMBER_RIGHT)))


def defs(tile_gradient=True):
    g = ('<linearGradient id="bf-ember" x1="0" y1="0" x2="0" y2="1">'
         f'<stop offset="0" stop-color="{EMBER_TOP_A}"/><stop offset="1" stop-color="{EMBER_TOP_B}"/>'
         '</linearGradient>')
    if tile_gradient:
        g += ('<linearGradient id="bf-tile" x1="0" y1="0" x2="0.5" y2="1">'
              f'<stop offset="0" stop-color="{GREEN_TOP}"/><stop offset="1" stop-color="{GREEN_BOTTOM}"/>'
              '</linearGradient>')
    return "<defs>" + g + "</defs>"


def mark():
    """Contents of the 512 icon, drawn inside the tile 16..496."""
    T = 286
    return (cube(304, T, 108, 8, 12) +
            f'<path d="{anvil_path(T)}" fill="{ANVIL}"/>' +
            f'<path d="{star4(138, 158, 48)}" fill="{SPARK}"/>' +
            f'<path d="{star4(428, 118, 22)}" fill="{SPARK}"/>')


def svg(w, h, body, title="BlockForge"):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">'
            f'<title>{title}</title>{body}</svg>\n')


def icon_svg():
    return svg(512, 512, defs() + '<rect x="16" y="16" width="480" height="480" rx="112" fill="url(#bf-tile)"/>'
               + mark())


def icon_small_svg():
    """24-32 px: full-bleed tile, no sparks or seams, bigger shapes, face edges on whole pixels at 32 px."""
    T, k = 288, 64 / 54
    return svg(512, 512, defs() + '<rect width="512" height="512" rx="104" fill="url(#bf-tile)"/>' +
               cube(304, T, 128, 0, 10) + f'<path d="{anvil_path(T, k, cx=247)}" fill="{ANVIL}"/>')


def icon_16_svg():
    """Hand placed pixels for 16x16."""
    Y, L, R, W = "url(#bf-ember)", EMBER_LEFT, EMBER_RIGHT, ANVIL
    rows = {
        3: [(7, 10, Y)],
        4: [(6, 11, Y)],
        5: [(6, 7, L), (8, 9, Y), (10, 11, R)],
        6: [(6, 8, L), (9, 11, R)],
        7: [(6, 8, L), (9, 11, R)],
        8: [(6, 8, L), (9, 11, R)],
        9: [(2, 13, W)],
        10: [(4, 13, W)],
        11: [(7, 10, W)],
        12: [(5, 12, W)],
    }
    rects = []
    for y, spans in rows.items():
        for x0, x1, col in spans:
            rects.append(f'<rect x="{x0}" y="{y}" width="{x1 - x0 + 1}" height="1" fill="{col}"/>')
    # soften the horn tip and its underside
    rects.append(f'<rect x="1" y="9" width="1" height="1" fill="{W}" opacity="0.55"/>')
    rects.append(f'<rect x="3" y="10" width="1" height="1" fill="{W}" opacity="0.55"/>')
    ember = ('<linearGradient id="bf-ember" gradientUnits="userSpaceOnUse" x1="0" y1="3" x2="0" y2="6">'
             f'<stop offset="0" stop-color="{EMBER_TOP_A}"/><stop offset="1" stop-color="{EMBER_TOP_B}"/>'
             '</linearGradient>')
    tile = ('<linearGradient id="bf-tile" x1="0" y1="0" x2="0.5" y2="1">'
            f'<stop offset="0" stop-color="{GREEN_TOP}"/><stop offset="1" stop-color="{GREEN_BOTTOM}"/>'
            '</linearGradient>')
    return svg(16, 16, "<defs>" + ember + tile + "</defs>" +
               '<rect width="16" height="16" rx="3.25" fill="url(#bf-tile)"/>' +
               '<g shape-rendering="crispEdges">' + "".join(rects) + "</g>")


def icon_macos_svg():
    """macOS grid: 824 px tile centred on a 1024 canvas (the drop shadow is added when rendering)."""
    s = 824 / 480
    return svg(1024, 1024, defs() + '<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bf-tile)"/>'
               + f'<g transform="translate(100 100) scale({f(s)}) translate(-16 -16)">{mark()}</g>')


# ---------------------------------------------------------------- wordmark (text converted to outlines)

class Text:
    def __init__(self, font_file):
        self.file = str(font_file)
        self.tt = TTFont(self.file)
        self.glyphs = self.tt.getGlyphSet()
        self.upm = self.tt["head"].unitsPerEm
        blob = hb.Blob.from_file_path(self.file)
        self.hb_font = hb.Font(hb.Face(blob))

    def shape(self, text):
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(self.hb_font, buf, {"kern": True, "liga": True})
        order = self.tt.getGlyphOrder()
        return [(order[i.codepoint], p.x_advance, p.x_offset, p.y_offset)
                for i, p in zip(buf.glyph_infos, buf.glyph_positions)]

    def width(self, text, size):
        return sum(a for _, a, _, _ in self.shape(text)) * size / self.upm

    def path(self, text, x, baseline, size):
        sc = size / self.upm
        out = []
        pen_x = 0
        for name, adv, dx, dy in self.shape(text):
            sp = SVGPathPen(self.glyphs, ntos=f)
            tp = TransformPen(sp, (sc, 0, 0, -sc, x + (pen_x + dx) * sc, baseline - dy * sc))
            self.glyphs[name].draw(tp)
            out.append(sp.getCommands())
            pen_x += adv
        return "".join(out)


BOLD = Text(FONTS / "Roboto-Bold.ttf")
REGULAR = Text(FONTS / "Roboto-Regular.ttf")


def wordmark(x, baseline, size, block_col, forge_col):
    w = BOLD.width("Block", size)
    return (f'<path d="{BOLD.path("Block", x, baseline, size)}" fill="{block_col}"/>'
            f'<path d="{BOLD.path("Forge", x + w, baseline, size)}" fill="{forge_col}"/>')


def lockup(theme, w, h, icon, x, title_size, sub_size, background=False):
    """Icon + 'BlockForge' + 'Minecraft launcher', vertically centred."""
    bg, block_col, forge_col, sub_col = THEMES[theme]
    cap = title_size * 0.711  # Roboto cap height
    gap = sub_size * 0.95
    block_h = cap + gap + sub_size * 0.74
    top = (h - block_h) / 2
    base1 = top + cap
    base2 = base1 + gap + sub_size * 0.74
    tx = x + icon + icon * 0.16
    s = icon / 480
    body = defs()
    if background:
        body += f'<rect width="{w}" height="{h}" fill="{bg}"/>'
    body += (f'<g transform="translate({f(x)} {f((h - icon) / 2)}) scale({f(s)}) translate(-16 -16)">'
             '<rect x="16" y="16" width="480" height="480" rx="112" fill="url(#bf-tile)"/>' + mark() + '</g>' +
             wordmark(tx + title_size * 0.02, base1, title_size, block_col, forge_col) +
             f'<path d="{REGULAR.path("Minecraft launcher", tx + sub_size * 0.08, base2, sub_size)}" '
             f'fill="{sub_col}"/>')
    return body, tx + BOLD.width("BlockForge", title_size)


def banner_svg(theme):
    w, h = 1280, 320
    icon, title, sub = 208, 112, 36
    _, right = lockup(theme, w, h, icon, 0, title, sub)
    x = (w - right) / 2
    body, _ = lockup(theme, w, h, icon, x, title, sub)
    return svg(w, h, body, "BlockForge, a Minecraft launcher")


def splash_svg():
    w, h = 576, 192
    bg = THEMES["dark"][0]
    glow = ('<radialGradient id="glow" cx="0.5" cy="0.5" r="0.5">'
            '<stop offset="0" stop-color="#36B964" stop-opacity="0.22"/>'
            '<stop offset="1" stop-color="#36B964" stop-opacity="0"/></radialGradient>'
            '<linearGradient id="bar" x1="0" y1="0" x2="1" y2="0">'
            f'<stop offset="0" stop-color="{GREEN_TOP}"/><stop offset="0.7" stop-color="{GREEN_TOP}"/>'
            f'<stop offset="1" stop-color="{EMBER_LEFT}"/></linearGradient>')
    body, _ = lockup("dark", w, h - 4, 120, 36, 56, 19)
    return svg(w, h, f'<defs>{glow}</defs><rect width="{w}" height="{h}" fill="{bg}"/>'
               f'<circle cx="96" cy="94" r="110" fill="url(#glow)"/>' + body +
               f'<rect y="{h - 4}" width="{w}" height="4" fill="url(#bar)"/>', "BlockForge splash screen")


# ---------------------------------------------------------------- rendering

def render(svg_text, w, h=None):
    png = cairosvg.svg2png(bytestring=svg_text.encode(), output_width=w, output_height=h or w)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def icon_at(size, full, small, px16):
    if size <= 16:
        return render(px16, 16).resize((size, size), Image.LANCZOS) if size < 16 else render(px16, 16)
    if size <= 32:
        return render(small, size)
    return render(full, size)


def macos(size, mac_svg):
    """macOS icon with a soft drop shadow under the tile."""
    S = max(size, 256)
    img = render(mac_svg, S)
    tile = Image.new("L", (S, S), 0)
    tile.paste(img.getchannel("A"))
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    a = tile.point(lambda v: int(v * 0.32)).filter(ImageFilter.GaussianBlur(S * 0.012))
    shadow.putalpha(a)
    shadow = shadow.transform((S, S), Image.AFFINE, (1, 0, 0, 0, 1, -S * 0.01))
    out = Image.alpha_composite(shadow, img)
    return out if S == size else out.resize((size, size), Image.LANCZOS)


def png_bytes(img):
    b = io.BytesIO()
    img.save(b, "PNG", optimize=True)
    return b.getvalue()


def write_icns(path, images):
    """images: {icns type: PIL image}. PNG payloads, with a table of contents."""
    entries = [(t.encode(), png_bytes(im)) for t, im in images.items()]
    toc = b"TOC " + struct.pack(">I", 8 + 8 * len(entries)) + b"".join(
        t + struct.pack(">I", 8 + len(d)) for t, d in entries)
    body = toc + b"".join(t + struct.pack(">I", 8 + len(d)) + d for t, d in entries)
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def write_ico(path, images):
    """images: list of PIL images, written as PNG entries (Vista+ format)."""
    datas = [png_bytes(im) for im in images]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    dirs = b""
    for im, d in zip(images, datas):
        w, h = im.size
        dirs += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    path.write_bytes(header + dirs + b"".join(datas))


def main():
    full, small, px16, mac = icon_svg(), icon_small_svg(), icon_16_svg(), icon_macos_svg()
    sources = {
        "blockforge-icon.svg": full,
        "blockforge-icon-small.svg": small,
        "blockforge-icon-16.svg": px16,
        "blockforge-icon-macos.svg": mac,
        "banner.svg": banner_svg("light"),
        "banner-dark.svg": banner_svg("dark"),
        "splash.svg": splash_svg(),
    }
    for name, text in sources.items():
        (LOGO / name).write_text(text)

    png_dir = LOGO / "png"
    png_dir.mkdir(exist_ok=True)
    for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
        icon_at(size, full, small, px16).save(png_dir / f"blockforge-{size}.png", optimize=True)
    for name in ("banner", "banner-dark"):
        render(sources[name + ".svg"], 1280, 320).save(LOGO / f"{name}.png", optimize=True)

    # launcher resources (same names and sizes as before)
    icon_at(256, full, small, px16).save(IMAGES / "icon.png", optimize=True)
    macos(1024, mac).save(IMAGES / "icon-osx.png", optimize=True)
    write_ico(IMAGES / "icon.ico", [icon_at(s, full, small, px16) for s in (16, 24, 32, 48, 64, 128, 256)])
    write_icns(IMAGES / "icon.icns", {
        "icp4": icon_at(16, full, small, px16),
        "icp5": icon_at(32, full, small, px16),
        "ic07": macos(128, mac),
        "ic08": macos(256, mac),
        "ic09": macos(512, mac),
        "ic10": macos(1024, mac),
        "ic11": icon_at(32, full, small, px16),
        "ic12": macos(64, mac),
        "ic13": macos(256, mac),
        "ic14": macos(512, mac),
    })
    render(sources["splash.svg"], 576, 192).convert("RGB").save(IMAGES / "splash-screen.png", optimize=True)

    # Linux packaging icons
    for d in LINUX_ICONS:
        if d.is_dir():
            icon_at(128, full, small, px16).save(d / "atlauncher.png", optimize=True)
            (d / "atlauncher.svg").write_text(full)
    print("done")


if __name__ == "__main__":
    main()
