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
type. Keys act on key-down. Audio dial clicks select devices; screen taps mute.
Theme/Network clicks switch profiles; their taps keep wallpaper/speed-test actions.

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
| Volume | Adjust output by 1% per tick | Click: next output device; tap: mute | Small device name, percentage, bar, mute/stale status |
| Mic | Adjust input by 1% per tick | Click: next input device; tap: mute | Small device name, percentage, bar, mute/stale status |
| Workspace | Next/previous workspace | Open Omarchy menu | Number, occupancy pips, active window |
| Theme | Next/previous theme | Click: performance profile; tap: next wallpaper | Theme name, palette swatches, list position |

Audio adjustments clamp to 0–100%; observation and mute do not change externally
set above-cap values. Workspace rotation uses the sign of an event, retaining
only the latest pending direction. Theme rotation drops repeats while busy.
Night and Record each share a busy gate across their duplicate instances.

Audio device clicks cycle currently available sinks or sources in stable node-name
order, wrapping at the end; a single device is a no-op. Selection changes the system's
configured default through WirePlumber and waits for confirmation. Rotation and mute
follow the selected device. Device labels prefer a short nickname, then description,
then node name, and are fitted to a small line above the percentage.

## System Stats page

The optional **System Stats** profile uses four additional dial actions:

| Panel | Live graph and values |
|---|---|
| CPU | Aggregate utilization across all cores, 0–100%, and current power profile |
| Memory | RAM utilization (`total − available`), used/total bytes |
| Disk | Physical-disk read/write throughput (`R`/`W`) |
| Network | Physical-interface receive/transmit throughput (`D`/`U`) |

Graphs update once per second with a rolling 60-sample history. Disk/network use
a shared auto-scale for their two traces: solid theme accent for read/download,
dashed foreground for write/upload. All graphs preserve grayscale themes and the
CRT frame, scanlines, and embedded font.

- **CPU rotate:** cycle available power profiles (clockwise next, anticlockwise
  previous), wrapping between Power Saver, Balanced, and Performance when all
  three are available. Each event uses its direction, not its tick count; repeats
  drop while a change is running. Omarchy remembers the choice for AC/battery use.
- **Network tap:** open Omarchy's speed-test overlay, which runs download and
  upload tests. **Dial-click:** switch to `default`. Rotation does nothing.
- **Memory and Disk:** remain read-only. CPU click/tap does nothing.

Click the rightmost dial to move between **default** (Theme) and **performance**
(Network). This uses OpenDeck's native `switchProfile` message, just like its
built-in Switch Profile action; no separate action needs to be placed on the dial.
Both named profiles must exist for the toggle.
Stock OpenDeck 2.14.0 restricts this message to built-in plugins; the combined
dial behavior requires the [local host permission patch](docs/opendeck-host-patch.md).

After installing the plugin, close OpenDeck and run:

```sh
python scripts/manage.py stats-profile
opendeck
```

Select **System Stats** in OpenDeck. Its keys are copied from the current Default
profile; the new graphs occupy its four dial panels. The command leaves Default
and its existing Volume/Mic/Workspace/Theme panels intact and refuses to overwrite
an existing System Stats profile. See [stats setup](docs/install.md#system-stats-profile).

## Requirements

- Linux x86-64, OpenDeck, Omarchy, and Hyprland in a working Wayland session.
- Rust/Cargo with Rust 2024 support and Python 3 for install/migration tooling.
- A running PipeWire/WirePlumber session, with the WirePlumber 0.5 runtime library
  and its `default-nodes-api`/`mixer-api` modules. Building requires a C compiler,
  `pkg-config`, and WirePlumber 0.5/GLib development headers (on Arch, `base-devel`
  and `wireplumber`). Audio uses a persistent native library connection.
- Desktop commands: `hyprctl`, `pgrep`, `ps`, and the Omarchy
  launch, capture, clipboard, nightlight, lock, theme, and menu commands.
- CPU profile control uses `omarchy-powerprofiles-list --active-state` and
  `omarchy-powerprofiles-set`, backed by `powerprofilesctl`. Network speed test
  requires the running Omarchy shell and its `omarchy.speedtest` panel.
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
a local WebSocket server, isolated desktop-command fixtures, and a private
PipeWire daemon. Native audio tests also use `wpctl` and `pw-metadata` as independent
test observers/controllers; neither is used by the plugin at runtime. Python tests
exercise migration and rollback in temporary homes with process calls mocked.

Generated binaries, images, Python caches, and IDE state are ignored by Git.

## Documentation

- [Install, migrate, and roll back](docs/install.md)
- [OpenDeck profile-switch permission patch](docs/opendeck-host-patch.md)
- [Architecture, runtime limits, verification, and known limitations](NOTES.md)
- [Product terminology](GLOSSARY.md)
- [Rust/OpenAction decision](docs/adr/0001-rust-and-openaction.md)
- [Historical research and design proposals](RESEARCH.md)

The original numbered design briefs are no longer present in this checkout.
Historical references to them in the research and source comments describe the
original design context; the guides above document the current implementation.
