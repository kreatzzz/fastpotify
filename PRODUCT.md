# Product

## Register

product

## Users

Spotify listeners who want a desktop client that starts instantly, stays
light, and stays honest: people who live on the keyboard, on Linux/macOS/Windows,
and — for the plugin story — listeners of music in scripts and languages not
their own (sing-along romanization, translated lyrics). Plugin authors are a
second audience: Rust developers shipping sandboxed `.wasm` providers.

## Product Purpose

Woofer is a fast, native Spotify client (Rust + egui + librespot): the whole
library, Connect control, synced lyrics with translation and romanization,
and an extendable plugin system whose catalog lives at usewoofer.com. Success
is a client that feels lighter and faster than the official one and never
pretends to be more than it is.

## Brand Personality

Quiet, precise, honest. Three words: **fast, minimal, trustworthy**. The
interface speaks in lowercase confidence — small secondary text, one green
accent, no decoration that does not work for its keep.

## Anti-references

- SaaS-cream marketing sites, glassmorphism, gradient text, glowing hero
  metrics — the AI-slop families generally.
- Spotify-clone vanity projects: no fake desktop-app chrome, no purple-blue
  gradients.
- Plugin-store gamification: no star ratings, badges, or countdowns in the
  catalog. The catalog is a reviewed list, not a bazaar.

## Design Principles

1. **Fast is the brand.** Every screen should feel like it loaded before you
   looked at it; no animation delays the first meaningful paint.
2. **Honest surfaces.** Empty states say what is happening ("the built-in
   source answers"); errors are one quiet line with a way forward.
3. **One accent, spent carefully.** Green marks the active, the actionable,
   and the alive — never decoration.
4. **Type does the hierarchy.** Weight and size over boxes and borders;
   cards appear only when they group something real.
5. **The sandbox is visible.** Anywhere a plugin appears, its declared
   domains and its review provenance are one glance away.

## Accessibility & Inclusion

- Body text ≥ 4.5:1 contrast on every surface (dark theme is the default;
  light theme exists and must be equally legible).
- Every animation honors `prefers-reduced-motion` with an instant state.
- Keyboard-first: every common action has a shortcut; focus is never trapped.
- Scripts beyond Latin are first-class (CJK, RTL, Indic shaping are merged
  upstream work — keep them working in every new surface).
