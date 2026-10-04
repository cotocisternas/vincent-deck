# vincent-deck — build brief

Design status: **confirmed by the user on 2026-10-03**, after the 27-decision
design interview. The numbered specifications are the product contract;
`RESEARCH.md` supplies evidence and provisional internal design. Engineering
limits are delegated for measurement and documentation within that contract.

This folder is a **specification, not an implementation**. It describes a single
OpenDeck plugin ("vincent-deck") that owns every key and dial of one Stream Deck,
draws all of their pictures itself, keeps them in sync with the desktop theme and
with live system state, and runs the actions when they are pressed or turned.

The implementation uses **Rust and OpenAction**, packaged as a program OpenDeck
can launch plus a manifest. Rendering libraries remain provisional until one key
and one panel demonstrate acceptable visual fidelity. Internal structure remains
an implementation choice. See `docs/adr/0001-rust-and-openaction.md`.

Code snippets in these documents are illustrative (message shapes, commands,
file layouts). They are not source to copy.

## Why this exists

Today the deck is driven by two stock plugins (a "run command" plugin and a
PipeWire volume plugin) and a generator script that bakes labels into PNG files.
That setup cannot:

- follow the desktop theme without regenerating files and restarting OpenDeck,
- show live state (current workspace, theme name, volume, mic mute, night light,
  recording),
- recolour the volume/mic dial icons (they are embedded copies owned by another
  plugin).

One plugin that owns everything removes all of those limits.

## Goals (in priority order)

1. **Parity**: preserve existing commands and normal key/dial behavior, with
   agreed refinements in doc 03: 100 % audio ceilings, consistent panel taps,
   and busy-input handling. Observation never alters externally set audio values.
2. **Theme-following**: when the Omarchy theme changes (from anywhere: the dial,
   the Omarchy menu, a CLI call), every icon and panel is redrawn in the new
   theme's colours normally within about a second after Omarchy finishes applying
   the theme, with no OpenDeck restart. Transient palette-read failures may delay
   convergence while the plugin retries.
3. **Live panels**: the four touch-strip dials show current state instead of
   instructions (see `03-actions-and-behavior.md`).
4. **Live keys**: Night Light and Record keys reflect whether they are on.
5. **Robustness**: it recovers from backend failures and transient connections
   while alive, survives OpenDeck restarts, and handles missing tools and
   monochrome themes. A plugin-process crash may require an OpenDeck restart.

## Agreed scope and appearance

- Key actions support keys; dial actions support dials. Rearrangement and
  duplicate instances are supported within each controller type.
- Preserve the existing CRT appearance perceptually, with exact dimensions and
  layout constants; minor rasterization differences are acceptable. Final visual
  acceptance is on the physical device.
- Monochrome themes remain monochrome. State must be recognizable through text,
  glyphs, and shape without injecting a colorful fallback palette.
- Unavailable state is visibly marked stale; it is never presented as confirmed
  off, idle, or muted. Last-known values may remain visibly dimmed and qualified.
- Full rollback remains available after cleanup through a complete legacy backup.

## Non-goals

- No settings UI / property inspector (the actions have no per-instance settings).
- No support for other devices, other OSes, or other desktops.
- Do not modify OpenDeck itself or other plugins' files.

## Documents

| File | What it covers |
|---|---|
| `01-environment.md` | The machine, the device, paths, commands that exist, constraints |
| `02-opendeck-protocol.md` | How a plugin talks to OpenDeck (verified facts + pitfalls) |
| `03-actions-and-behavior.md` | Every action: what it shows, what it does |
| `04-visual-spec.md` | Pixel-level look of keys and panels, palette, fonts |
| `05-live-state.md` | Where each piece of live state comes from, refresh rules |
| `06-install-migrate-verify.md` | Manifest, install, profile migration, rollback, acceptance tests, cleanup |
| `RESEARCH.md` | Supporting evidence and proposed implementation; numbered specs take precedence |
| `GLOSSARY.md` | Agreed product terminology |

Read them in that order.
