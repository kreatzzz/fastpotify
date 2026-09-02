# Design

## Native app theme

Dark-first, near-black. The scene: one person, headphones on, room lights
low, the client open beside whatever else they are doing. Light theme exists
and mirrors every token inverted, equally legible.

## Native app color palette (dark)

| Token | Value | Use |
| --- | --- | --- |
| `--bg` | `#121212` | app background, site background |
| `--panel` | `#1b1b1b` | cards, sections |
| `--panel-hover` | `#232323` | hover on cards/rows |
| `--surface` | `#181818` | inset fields, inputs |
| `--text` | `#f0f0f0` | primary ink |
| `--secondary` | `#a7a7a7` | secondary ink (≥ 7:1 on bg) |
| `--dim` | `#8a8a8a` | quietest ink, never below 4.5:1 on bg |
| `--accent` | `#1ed760` | active, actionable, alive |
| `--accent-text` | `#08290f` | ink on accent |
| `--outline` | `#2e2e2e` | 1px borders |

Strategy: **restrained** — tinted near-neutrals, one green accent under 10%
of any surface. Light theme mirrors with `#ffffff` page, `#f6f6f6` panels,
ink `#121212`/`#5a5a5a`, same accent darkened for contrast (`#12943f` on
white).

## Native app typography

- Stack: `-apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif`
  (the app embeds Inter).
- Scale: 28 page titles (bold), 18 section titles (bold), 14–15 body,
  13 secondary, 12–12.5 captions. Line-height 1.6 prose, 1.3 UI.
- Prose measure ≤ 72ch. `text-wrap: balance` on headings.
- No font pairing — one family, weights carry hierarchy.

## Native app components

- **Pill buttons**: 999px radius, accent fill (primary) or 1px outline
  (ghost), padding 9px 18px; hover = brightness/border shift, 150ms
  ease-out.
- **Cards/sections**: 14px radius, `--panel` fill, 1px `--outline` border,
  22-24px padding. Never a border + wide shadow together.
- **Rows (settings)**: label 14 medium + description 13 secondary capped at
  ~56ch, control right-aligned, 12px vertical rhythm between rows.
- **Chips**: 999px, 1px outline, 12px text, quiet.
- **Switches**: 40×22 pill, white knob, accent when on.
- **Focus**: 2px accent outline, offset 2px, on every interactive element.

## Native app layout

- App: sidebar (library) + main column; panels overlay from the right.
- Spacing scale: 4 / 8 / 12 / 16 / 24 / 40 / 64. Vary rhythm by section
  weight; never uniform card grids.

## Native app motion

- Energy: near-none. 150–220ms ease-out for hover/press and state fills;
  the lyrics line light-up is the one showpiece (220ms).
- No entrance staggering, no parallax, no bounce.

## Marketing-site direction

The docs and landing site translate Woofer's speed and trust into a more
playful, music-led register. The physical scene is a listener at a desk with
headphones on, a low lamp, and one song moving between the laptop and a
speaker. The site should feel like a hand-made record sleeve pinned beside
that desk: confident type, real product screenshots, a few bright notes, and
room to breathe.

### Site palette

The site keeps a dark-first canvas but carries more colour than the app. Use
OKLCH tokens so the light theme can mirror the same relationships without
losing contrast.

| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `--site-bg` | `oklch(0.14 0.018 150)` | `oklch(0.97 0 0)` | page canvas |
| `--site-panel` | `oklch(0.20 0.022 150)` | `oklch(0.93 0.018 150)` | section bands |
| `--site-ink` | `oklch(0.97 0 0)` | `oklch(0.20 0.025 150)` | body and headings |
| `--site-muted` | `oklch(0.78 0.02 145)` | `oklch(0.36 0.025 150)` | supporting copy |
| `--site-lime` | `oklch(0.86 0.20 134)` | `oklch(0.48 0.16 145)` | primary action, active notes |
| `--site-coral` | `oklch(0.73 0.17 35)` | `oklch(0.56 0.16 35)` | lyric-led accent |
| `--site-lilac` | `oklch(0.78 0.12 288)` | `oklch(0.55 0.11 288)` | secondary accent |

Lime remains the action colour. Coral and lilac are small, semantic notes for
the music-led story; they should not become a rainbow UI or compete with
download and navigation. Never use gradient text, decorative grid overlays,
or a repeated card matrix.

### Site type and layout

The landing page uses a rounded display face for short, memorable phrases and
an accessible humanist sans for prose. Headings use `clamp()` with a 6rem
ceiling, `text-wrap: balance`, and letter-spacing no tighter than `-0.04em`.
Guide prose remains at 65–75ch with `text-wrap: pretty`. The hero is a
deliberately tilted screenshot and the content rhythm alternates between
full-width image bands and simple signal rows rather than nested cards.

The docs keep the familiar VitePress navigation pattern: local search, a
collapsible sidebar, breadcrumbs on mobile, and a dark/light appearance
toggle. The established public page URLs are preserved while source files
stay grouped in `_guide/` and `_reference/` until the docs workflow migrates.

### Site motion and inclusion

Motion is limited to a small hero reveal, image tilt, and button lift. Every
transition has a `prefers-reduced-motion: reduce` path that removes movement.
Focus rings use the lime token with a visible offset, links keep underlines,
and muted copy stays above 4.5:1 contrast in both themes. All screenshot
captions describe what a listener can learn from the image; they do not claim
an account state that a visitor cannot verify.
