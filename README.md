# Vincent Deck

A native [OpenDeck](https://github.com/nekename/OpenDeck) plugin for an Elgato
Stream Deck + on an Omarchy/Hyprland desktop. It draws CRT-style keys and dial
panels, follows the desktop palette, shows live system state, and launches desktop
controls asynchronously.

Implemented in Rust 2024 with OpenAction 2.7.0, Tokio, tiny-skia, and fontdue.
The plugin is deployed on the author's desktop; basic operation and appearance
have been approved. Full hardware acceptance and performance measurements remain
open. See [implementation notes](NOTES.md) for verification and known limitations.

## Controls

All actions can be rearranged or duplicated within their supported controller
type. Keys act on key-down; dial press and panel tap perform the same action.

| Key action | Press | Live display |
|---|---|---|
| Terminal | Open terminal | `TTY` |
| Browser | Open browser | `NET` |
| Screenshot | Capture screenshot | `SNAP` |
| Record | Toggle screen recording | Elapsed time, pending, or stale |
| Agent | Toggle the default agent scratchpad, matching Mod+grave | `AGENT` |
| Clipboard | Open clipboard history | `CLIP` |
| Night | Toggle Night Light | Confirmed on/off, pending, or stale |
| Lock | Lock the session | Confirmed locked, pending, or stale |

| Dial action | Rotate | Press / panel tap | Live display |
|---|---|---|---|
| Volume | Adjust output by 1% per tick | Toggle output mute | Percentage, bar, mute/stale status |
| Mic | Adjust input by 1% per tick | Toggle input mute | Percentage, bar, mute/stale status |
| Workspace | Next/previous workspace | Open Omarchy menu | Number, occupancy pips, active window |
| Theme | Next/previous theme | Next wallpaper | Theme name, palette swatches, list position |

Audio adjustments clamp to 0–100%; observation and mute do not change externally
set above-cap values. Workspace rotation uses the sign of an event, retaining
only the latest pending direction. Theme rotation drops repeats while busy.
Night and Record each share a busy gate across their duplicate instances.

## Requirements

- Linux x86-64, OpenDeck, Omarchy, and Hyprland in a working Wayland session.
- Rust/Cargo with Rust 2024 support and Python 3 for install/migration tooling.
- Desktop commands: `wpctl`, `pactl`, `hyprctl`, `pgrep`, `ps`, and the Omarchy
  launch, capture, clipboard, nightlight, lock, theme, and menu commands.
- Existing `~/.local/bin/deck-workspace` and `deck-theme-cycle` helpers. This
  repository does not install those helpers.

Migration tooling targets device `sd-EL31L1A08599`, eight keys, four dials, and its
`Default` profile. Other device IDs require adapting `scripts/manage.py`.

Terminess Nerd Font Mono Bold is bundled in `assets/fonts/` and embedded into the
executable at compile time. Neither the plugin nor the tests require a system font
installation or a runtime font file. The font retains its SIL OFL 1.1 license;
see [font provenance and licensing](assets/fonts/README.md).

## Build and install

Run from the repository root:

```sh
python scripts/manage.py install
```

This runs `cargo build --release`, renders sample icons, generates the manifest,
backs up an existing installed bundle, and installs to
`~/.config/opendeck/plugins/dev.vincent.deck.sdPlugin/`.
The generated bundle is also available under `dist/`.

Restart OpenDeck to load the installed binary. Close OpenDeck and wait for its
plugin processes to exit, then launch it again:

```sh
opendeck
```

The executable normally receives its connection arguments from OpenDeck; it is
not a standalone desktop application. Build alone with `cargo build --release`.

For test-profile creation, migration, and rollback, see the
[installation guide](docs/install.md).

## Development checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
python -m unittest discover -s tests -p 'test_*.py'
```

The host harness runs the real plugin against
a local WebSocket server and isolated desktop-command fixtures. Python tests
exercise migration and rollback in temporary homes with process calls mocked.

Generated binaries, images, Python caches, and IDE state are ignored by Git.

## Documentation

- [Install, migrate, and roll back](docs/install.md)
- [Architecture, runtime limits, verification, and known limitations](NOTES.md)
- [Product terminology](GLOSSARY.md)
- [Rust/OpenAction decision](docs/adr/0001-rust-and-openaction.md)
- [Historical research and design proposals](RESEARCH.md)

The original numbered design briefs are no longer present in this checkout.
Historical references to them in the research and source comments describe the
original design context; the guides above document the current implementation.
