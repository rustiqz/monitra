# Monitra — Cursor Eye logo pack

An owl whose right pupil is a terminal block cursor: the one eye that never closes.
All files are SVG. The wordmark is converted to outlines (Geist SemiBold, −0.035em tracking), so nothing depends on installed fonts.

## Which file to use

| Need | File |
|---|---|
| README header (GitHub) | `social/monitra-readme-banner-dark.svg` + `-light.svg` (see snippet below) |
| Link previews (og:image) | `social/monitra-og-image.svg` → export to PNG 1200×630, most platforms reject SVG og images |
| Browser tab | `favicon/favicon.svg` (adapts to light/dark browser UI) |
| Fallback favicons | `favicon/favicon-32-*.svg`, `favicon/favicon-16-*.svg` (heavier strokes; 16px drops the beak) |
| Dashboard / docs nav | `logo/monitra-logo-horizontal-dark.svg` |
| Hero / splash / about | `logo/monitra-logo-stacked-*.svg` |
| GitHub org avatar, iOS, Android | `icon/monitra-icon-*-square.svg` (platform applies its own corner mask) |
| Desktop, PWA, docs | `icon/monitra-icon-*-rounded.svg` |
| Just the symbol | `mark/monitra-mark-*.svg` |
| Text-only contexts | `wordmark/monitra-wordmark-*.svg` |
| Print, engraving, one-colour | any `*-mono-black` / `*-mono-white` |
| Live feel (dashboard loader, site header) | `*-animated.svg`: cursor blinks; stops automatically when the user has reduced motion enabled |

**Colour variants:** `dark` = for dark backgrounds, `light` = for light backgrounds.

## Colours

| Token | Hex | Use |
|---|---|---|
| Ink | `#0a0c0b` | Ground; mark on light |
| Bone | `#e6ece8` | Mark and wordmark on dark |
| Signal green | `#39ff14` | The cursor eye on dark backgrounds only |
| Deep green | `#1a9c00` | The cursor eye on light backgrounds (neon green is unreadable on white) |
| Muted | `#8b968f` | Secondary text on dark |

Rule of thumb: green appears **once**, on the cursor eye. Everything else stays ink/bone.

## Usage

- **Clear space:** keep at least the height of one eye (≈ 20% of mark height) empty around the logo.
- **Minimum size:** mark 16px (use the favicon-16 file below 24px); horizontal logo 96px wide.
- Don't recolour the left pupil green, rotate, outline, add shadows/gradients, or put the neon version on light backgrounds.

## GitHub README snippet

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/monitra-readme-banner-dark.svg">
  <img alt="Monitra" src="assets/monitra-readme-banner-light.svg">
</picture>
```

## HTML head snippet

```html
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon-32-dark.png" sizes="32x32"> <!-- optional PNG fallback -->
<meta property="og:image" content="https://your-domain/og.png">
```

## Regenerating

`_source/build_pack.py` builds every file from one set of geometry. It expects the Geist TTFs (`npm pack geist`) and `pip install uharfbuzz fonttools`; adjust `FONT_DIR` at the top.
