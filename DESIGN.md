# Design

## Theme

Dark-first, near-black. The scene: one person, headphones on, room lights
low, the client open beside whatever else they are doing. Light theme exists
and mirrors every token inverted, equally legible.

## Color Palette (dark)

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

## Typography

- Stack: `-apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif`
  (the app embeds Inter; the site uses the system stack).
- Scale: 28 page titles (bold), 18 section titles (bold), 14–15 body,
  13 secondary, 12–12.5 captions. Line-height 1.6 prose, 1.3 UI.
- Prose measure ≤ 72ch. `text-wrap: balance` on headings.
- No font pairing — one family, weights carry hierarchy.

## Components

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

## Layout

- App: sidebar (library) + main column; panels overlay from the right.
- Site: single centered column, `max-width: 860px`, 64px top padding;
  guide content column ≤ 72ch.
- Spacing scale: 4 / 8 / 12 / 16 / 24 / 40 / 64. Vary rhythm by section
  weight; never uniform card grids.

## Motion

- Energy: near-none. 150–220ms ease-out for hover/press and state fills;
  the lyrics line light-up is the one showpiece (220ms).
- Site: one load fade-up (12px, 400ms ease-out-quart) on hero content only,
  suppressed under `prefers-reduced-motion: reduce`.
- No entrance staggering, no parallax, no bounce.
