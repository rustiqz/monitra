#!/usr/bin/env python3
"""
Monitra "Cursor Eye" logo pack generator.

Builds every SVG in the brand pack from one set of geometry so all files stay
in sync. The wordmark is converted to outlines (paths) from Geist SemiBold,
so the SVGs render identically everywhere without the font installed.

Output layout (under ./monitra-logo-pack):
  mark/       the owl symbol alone (static + animated)
  logo/       horizontal and stacked lockups (mark + wordmark)
  wordmark/   the "monitra" wordmark alone
  icon/       app icons (rounded and full-bleed square)
  favicon/    adaptive favicon + size-tuned 32px / 16px versions
  social/     Open Graph image and README banners
"""
import os
import uharfbuzz as hb
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.transformPen import TransformPen

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "monitra-logo-pack")
FONT_DIR = os.path.join(HERE, "fonts/package/dist/fonts/geist-sans")

# ---------------------------------------------------------------------------
# Brand colours
# ---------------------------------------------------------------------------
INK = "#0a0c0b"          # near-black ground
BONE = "#e6ece8"         # off-white for marks on dark
GREEN = "#39ff14"        # electric green accent (dark backgrounds only)
GREEN_DEEP = "#1a9c00"   # accent on light backgrounds (neon fails contrast on white)
MUTED = "#8b968f"        # secondary text on dark
MUTED_LIGHT = "#5b665f"  # secondary text on light
LIGHT_BG = "#f2f4f1"     # light ground for banners

# Colour schemes for the mark: stroke/fill of the owl, left pupil, cursor pupil.
SCHEMES = {
    "dark":       dict(main=BONE, pupil=BONE, accent=GREEN),        # for dark backgrounds
    "light":      dict(main=INK, pupil=INK, accent=GREEN_DEEP),     # for light backgrounds
    "mono-white": dict(main="#ffffff", pupil="#ffffff", accent="#ffffff"),
    "mono-black": dict(main="#000000", pupil="#000000", accent="#000000"),
}

# ---------------------------------------------------------------------------
# Mark geometry (64 x 64 design grid)
# ---------------------------------------------------------------------------
# Visual bounds of the standard mark incl. stroke caps/miters, used to crop
# the mark tightly inside lockups and to centre it in icons.
MARK_BOX = (3.0, 7.0, 58.0, 49.0)  # x, y, w, h
MARK_CX = MARK_BOX[0] + MARK_BOX[2] / 2
MARK_CY = MARK_BOX[1] + MARK_BOX[3] / 2

# Blink keyframes: on for 55% of the cycle, then off — a terminal cursor.
# Disabled for users who ask the OS for reduced motion.
BLINK_CSS = (
    "@keyframes blink{0%,55%{opacity:1}56%,100%{opacity:0}}"
    ".cursor{animation:blink 1.1s steps(1,end) infinite}"
    "@media (prefers-reduced-motion:reduce){.cursor{animation:none}}"
)


def mark_elems(c, weight="regular", animated=False, beak=True):
    """Return the SVG elements of the owl mark in the 64-unit grid.

    weight: 'regular' for normal use, 'heavy' for small sizes (favicons),
            'tiny' for 16px (thicker again, no beak).
    """
    sw, pupil_r, cur = {
        "regular": (4.0, 3.5, (41.0, 31.0, 6.0, 12.0)),
        "heavy":   (5.5, 4.5, (40.5, 30.0, 7.0, 14.0)),
        "tiny":    (6.5, 5.0, (40.0, 29.5, 8.0, 15.0)),
    }[weight]
    cls = ' class="cursor"' if animated else ""
    cx, cy, cw, ch = cur
    parts = [
        # Brow: the angled line over the eyes, which also reads as an "M"
        f'<polyline points="6,10 20,19 32,13 44,19 58,10" fill="none" stroke="{c["main"]}" '
        f'stroke-width="{sw}" stroke-linecap="square" stroke-linejoin="miter"/>',
        # Eyes: two touching rings
        f'<circle cx="20" cy="37" r="10" fill="none" stroke="{c["main"]}" stroke-width="{sw}"/>',
        f'<circle cx="44" cy="37" r="10" fill="none" stroke="{c["main"]}" stroke-width="{sw}"/>',
        # Left pupil: a plain dot
        f'<circle cx="20" cy="37" r="{pupil_r}" fill="{c["pupil"]}"/>',
        # Right pupil: the block cursor, i.e. the "always watching" eye
        f'<rect{cls} x="{cx}" y="{cy}" width="{cw}" height="{ch}" fill="{c["accent"]}"/>',
    ]
    if beak:
        b = (28.5, 50, 35.5, 50, 32, 56) if weight == "regular" else (28, 50.5, 36, 50.5, 32, 57)
        parts.append(f'<polygon points="{b[0]},{b[1]} {b[2]},{b[3]} {b[4]},{b[5]}" fill="{c["main"]}"/>')
    return "\n    ".join(parts)


# ---------------------------------------------------------------------------
# Wordmark: Geist text shaped with HarfBuzz, outlined with fontTools
# ---------------------------------------------------------------------------
class Outliner:
    def __init__(self, ttf_path):
        self.font = TTFont(ttf_path)
        self.gs = self.font.getGlyphSet()
        self.upem = self.font["head"].unitsPerEm
        self.xh = self.font["OS/2"].sxHeight
        data = open(ttf_path, "rb").read()
        self.hbfont = hb.Font(hb.Face(hb.Blob(data)))

    def outline(self, text, tracking_em=0.0):
        """Return (path_d, bounds) in font units, y flipped to SVG (down = +)."""
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(self.hbfont, buf, {"kern": True, "liga": True})
        order = self.font.getGlyphOrder()
        pen = SVGPathPen(self.gs)
        bpen = BoundsPen(self.gs)
        x = 0
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            name = order[info.codepoint]
            # Flip y so the outline is in SVG coordinates (baseline at y=0).
            t = (1, 0, 0, -1, x + pos.x_offset, -pos.y_offset)
            self.gs[name].draw(TransformPen(pen, t))
            self.gs[name].draw(TransformPen(bpen, t))
            x += pos.x_advance + tracking_em * self.upem
        return pen.getCommands(), bpen.bounds  # bounds: xMin, yMin, xMax, yMax


SEMI = Outliner(os.path.join(FONT_DIR, "Geist-SemiBold.ttf"))
REG = Outliner(os.path.join(FONT_DIR, "Geist-Regular.ttf"))
WORD_D, WORD_B = SEMI.outline("monitra", tracking_em=-0.035)


def word_group(font_px, x, baseline, fill):
    """Place the outlined wordmark: left edge of its ink at x, baseline at y."""
    s = font_px / SEMI.upem
    tx = x - WORD_B[0] * s
    return f'<path transform="translate({tx:.2f} {baseline:.2f}) scale({s:.5f})" fill="{fill}" d="{WORD_D}"/>'


def word_metrics(font_px):
    """Ink width, ink top (above baseline, negative) and x-height for a size."""
    s = font_px / SEMI.upem
    return (WORD_B[2] - WORD_B[0]) * s, WORD_B[1] * s, SEMI.xh * s


def text_group(outliner, text, font_px, cx, baseline, fill, tracking=0.0):
    """Centred line of outlined secondary text (tagline etc.)."""
    d, b = outliner.outline(text, tracking)
    s = font_px / outliner.upem
    tx = cx - (b[0] + b[2]) / 2 * s
    return f'<path transform="translate({tx:.2f} {baseline:.2f}) scale({s:.5f})" fill="{fill}" d="{d}"/>'


# ---------------------------------------------------------------------------
# SVG writers
# ---------------------------------------------------------------------------
def svg(w, h, body, title, comment, style=None, vb=None):
    vb = vb or f"0 0 {fmt(w)} {fmt(h)}"
    st = f"\n  <style>{style}</style>" if style else ""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{fmt(w)}" height="{fmt(h)}" viewBox="{vb}" '
        f'role="img" aria-labelledby="t">\n'
        f"  <!-- {comment} -->\n"
        f"  <title id=\"t\">{title}</title>{st}\n  {body}\n</svg>\n"
    )


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def write(rel, content):
    p = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write(content)


def placed_mark(c, size, x, y, **kw):
    """Mark scaled so its visual box is `size` tall, top-left of the box at (x, y)."""
    s = size / MARK_BOX[3]
    tx = x - MARK_BOX[0] * s
    ty = y - MARK_BOX[1] * s
    return f'<g transform="translate({tx:.2f} {ty:.2f}) scale({s:.4f})">\n    {mark_elems(c, **kw)}\n  </g>'


def mark_width(size):
    return MARK_BOX[2] * size / MARK_BOX[3]


# --- 1. Mark -----------------------------------------------------------------
for name, c in SCHEMES.items():
    write(f"mark/monitra-mark-{name}.svg", svg(
        256, 256, f"<g>\n    {mark_elems(c)}\n  </g>", "Monitra",
        f"Monitra mark ({name}). 64-unit grid, scales freely.", vb="0 0 64 64"))
for name in ("dark", "light"):
    write(f"mark/monitra-mark-{name}-animated.svg", svg(
        256, 256, f"<g>\n    {mark_elems(SCHEMES[name], animated=True)}\n  </g>", "Monitra",
        f"Monitra mark ({name}) with blinking cursor eye. Respects prefers-reduced-motion.",
        style=BLINK_CSS, vb="0 0 64 64"))


# --- 2. Lockups -------------------------------------------------------------
def horizontal(c, word_fill, animated=False):
    H = 64.0                          # mark visual height
    fs = H * 1.10                     # wordmark font size relative to mark
    gap = H * 0.40                    # space between mark and wordmark
    pad = 4.0                         # tiny safety margin around the artwork
    ww, top, xh = word_metrics(fs)
    mw = mark_width(H)
    # Vertically: put the middle of the x-height band (nudged towards the ink
    # box centre for the ascenders) on the mark's centre line.
    mid = pad + H / 2
    baseline = mid + (xh / 2 + (-top) / 2) / 2
    width = pad + mw + gap + ww + pad
    height = H + pad * 2
    body = placed_mark(c, H, pad, pad, animated=animated) + "\n  " + word_group(fs, pad + mw + gap, baseline, word_fill)
    return width, height, body


def stacked(c, word_fill, animated=False):
    H = 96.0
    fs = 76.0
    gap = 22.0
    pad = 4.0
    ww, top, xh = word_metrics(fs)
    mw = mark_width(H)
    width = max(mw, ww) + pad * 2
    baseline = pad + H + gap + (-top)
    height = baseline + pad + 2
    body = (placed_mark(c, H, (width - mw) / 2, pad, animated=animated) + "\n  "
            + word_group(fs, (width - ww) / 2, baseline, word_fill))
    return width, height, body


WORD_FILL = {"dark": BONE, "light": INK, "mono-white": "#ffffff", "mono-black": "#000000"}
for name, c in SCHEMES.items():
    w, h, b = horizontal(c, WORD_FILL[name])
    write(f"logo/monitra-logo-horizontal-{name}.svg",
          svg(w, h, b, "Monitra", f"Monitra horizontal logo ({name}). Wordmark is outlined Geist SemiBold."))
    w, h, b = stacked(c, WORD_FILL[name])
    write(f"logo/monitra-logo-stacked-{name}.svg",
          svg(w, h, b, "Monitra", f"Monitra stacked logo ({name}). Wordmark is outlined Geist SemiBold."))
for name in ("dark", "light"):
    w, h, b = horizontal(SCHEMES[name], WORD_FILL[name], animated=True)
    write(f"logo/monitra-logo-horizontal-{name}-animated.svg",
          svg(w, h, b, "Monitra", f"Monitra horizontal logo ({name}), blinking cursor eye.", style=BLINK_CSS))

# --- 3. Wordmark ------------------------------------------------------------
for name, fill in WORD_FILL.items():
    fs = 80.0
    ww, top, xh = word_metrics(fs)
    pad = 4.0
    write(f"wordmark/monitra-wordmark-{name}.svg",
          svg(ww + pad * 2, -top + pad * 2 + 2, word_group(fs, pad, pad - top, fill), "Monitra",
              f"Monitra wordmark ({name}). Outlined Geist SemiBold, -0.035em tracking."))


# --- 4. App icons (512 x 512) ----------------------------------------------
def icon(bg, c, radius, mark_w=300, weight="regular"):
    size = mark_w * MARK_BOX[3] / MARK_BOX[2]
    x = (512 - mark_w) / 2
    y = (512 - size) / 2 + 6          # optical nudge: brow is lighter than the beak end
    shape = f'<rect width="512" height="512" rx="{radius}" fill="{bg}"/>'
    return shape + "\n  " + placed_mark(c, size, x, y, weight=weight)


ON_GREEN = dict(main=INK, pupil=INK, accent=INK)
write("icon/monitra-icon-dark-rounded.svg", svg(512, 512, icon(INK, SCHEMES["dark"], 112), "Monitra",
      "App icon, dark, rounded corners baked in (web, desktop, docs)."))
write("icon/monitra-icon-green-rounded.svg", svg(512, 512, icon(GREEN, ON_GREEN, 112), "Monitra",
      "App icon, green, rounded corners baked in."))
write("icon/monitra-icon-dark-square.svg", svg(512, 512, icon(INK, SCHEMES["dark"], 0), "Monitra",
      "App icon, dark, full-bleed square: for platforms that apply their own mask (GitHub avatar, iOS, Android)."))
write("icon/monitra-icon-green-square.svg", svg(512, 512, icon(GREEN, ON_GREEN, 0), "Monitra",
      "App icon, green, full-bleed square."))
write("icon/monitra-icon-light-rounded.svg", svg(512, 512, icon(LIGHT_BG, SCHEMES["light"], 112), "Monitra",
      "App icon, light, rounded corners baked in."))

# --- 5. Favicons -------------------------------------------------------------
# Adaptive favicon: ink mark for light browser UI, bone + neon for dark UI.
FAV_CSS = (
    ".m{fill:none;stroke:%s}.p{fill:%s}.a{fill:%s}"
    "@media (prefers-color-scheme:dark){.m{stroke:%s}.p{fill:%s}.a{fill:%s}}"
    % (INK, INK, GREEN_DEEP, BONE, BONE, GREEN)
)
fav_body = (
    '<polyline class="m" points="6,10 20,19 32,13 44,19 58,10" stroke-width="5.5" '
    'stroke-linecap="square" stroke-linejoin="miter"/>\n  '
    '<circle class="m" cx="20" cy="37" r="10" stroke-width="5.5"/>\n  '
    '<circle class="m" cx="44" cy="37" r="10" stroke-width="5.5"/>\n  '
    '<circle class="p" cx="20" cy="37" r="4.5"/>\n  '
    '<rect class="a" x="40.5" y="30" width="7" height="14"/>\n  '
    '<polygon class="p" points="28,50.5 36,50.5 32,57"/>'
)
write("favicon/favicon.svg", svg(64, 64, fav_body, "Monitra",
      "Adaptive favicon: switches colours with the browser's light/dark theme. "
      "Use with <link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\">.",
      style=FAV_CSS))
for name in ("dark", "light"):
    write(f"favicon/favicon-32-{name}.svg", svg(32, 32, mark_elems(SCHEMES[name], weight="heavy"), "Monitra",
          f"32px favicon ({name}): heavier strokes for small sizes.", vb="0 0 64 64"))
    write(f"favicon/favicon-16-{name}.svg", svg(16, 16, mark_elems(SCHEMES[name], weight="tiny", beak=False),
          "Monitra", f"16px favicon ({name}): heaviest strokes, beak dropped for legibility.", vb="0 0 64 64"))

# --- 6. Social / banners -----------------------------------------------------
TAGLINE = "Self-hosted uptime monitoring. One binary, zero infrastructure."


def banner(w, h, bg, scheme, word_fill, tag_fill, logo_scale, tag_px, border=None):
    lw, lh, lbody = horizontal(SCHEMES[scheme], word_fill)
    s = logo_scale
    total_h = lh * s + tag_px * 1.9
    lx = (w - lw * s) / 2
    ly = (h - total_h) / 2
    parts = [f'<rect width="{w}" height="{h}" fill="{bg}"/>']
    if border:
        parts.append(f'<rect x="0.5" y="0.5" width="{w-1}" height="{h-1}" fill="none" stroke="{border}"/>')
    parts.append(f'<g transform="translate({lx:.2f} {ly:.2f}) scale({s})">\n  {lbody}\n  </g>')
    parts.append(text_group(REG, TAGLINE, tag_px, w / 2, ly + lh * s + tag_px * 1.9, tag_fill))
    return "\n  ".join(parts)


write("social/monitra-og-image.svg", svg(1200, 630, banner(1200, 630, INK, "dark", BONE, MUTED, 1.9, 30), "Monitra",
      "Open Graph / social preview image, 1200x630."))
write("social/monitra-readme-banner-dark.svg", svg(1280, 320, banner(1280, 320, INK, "dark", BONE, MUTED, 1.25, 22), "Monitra",
      "README header banner for dark themes, 1280x320."))
write("social/monitra-readme-banner-light.svg", svg(1280, 320, banner(1280, 320, LIGHT_BG, "light", INK, MUTED_LIGHT, 1.25, 22), "Monitra",
      "README header banner for light themes, 1280x320."))

print("done")
