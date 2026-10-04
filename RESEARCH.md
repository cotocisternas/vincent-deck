# Vincent Deck — research and proposed architecture

**Historical design research.** This document preserves the original proposals
and source evidence. The implementation now exists; use [README.md](README.md),
[NOTES.md](NOTES.md), and [docs/install.md](docs/install.md) for current behavior,
installation, verification, and known limitations. References below to the
original numbered briefs are historical; those files are no longer in this
checkout. Proposed interfaces and timing targets are not claims that every part
is implemented or verified.

Research date: 2026-10-03.
Target host: OpenDeck 2.14.0 on Arch/Omarchy (Hyprland, Wayland), Elgato Stream Deck + (`sd-EL31L1A08599`).
This document is the design brief for a single native Rust plugin that replaces the current mix of Starter Pack “run command” keys and the installed PipeWire volume/mic dials, while keeping the CRT tile look from `~/.local/bin/deck-icons`.

Claims below are pinned to a primary source: official docs, first-party source, or this machine. Local numbered specs (`01`–`06`) are treated as the product contract.

Design interview update, 2026-10-03: Rust and OpenAction are accepted (see
`docs/adr/0001-rust-and-openaction.md`). Rendering libraries and internal interfaces
below remain proposals. Agreed requirements now include within-type placement and
duplicates, perceptual visual fidelity, strict monochrome palettes, 100 % dial
ceilings without changing externally set values merely by observing them, visible
stale state, automatic backend recovery, and complete rollback after cleanup.

---

## 1. What we are building

One OpenDeck plugin process that owns all twelve deck slots:

| Slot | Action | Press / rotate |
|---|---|---|
| Keypad 0–7 | Terminal, Browser, Screenshot, Record, Agent, Clipboard, Night, Lock | `keyDown` runs an Omarchy command |
| Encoder 0 | Volume | 1 % ticks on the default sink; press/tap mute |
| Encoder 1 | Mic | same for the default source |
| Encoder 2 | Workspace | `deck-workspace`; press opens the menu |
| Encoder 3 | Theme | `deck-theme-cycle`; press next wallpaper |

Every picture follows the current theme. Night and Record have live variants. Volume, Mic, and Workspace update when the world changes outside the deck. Theme changes normally redraw everything within about one second after Omarchy finishes applying the theme, without restarting OpenDeck; transient palette-read failures may delay convergence.

The user-facing look is already decided in `04-visual-spec.md` and the current ImageMagick generator. This plugin must reproduce it, not redesign it.

---

## 2. Sources

### Local contract (this repo)

- `01-environment.md` — machine, device, OpenDeck paths, Omarchy commands, fonts.
- `02-opendeck-protocol.md` — process model, WebSocket, contexts, `setImage` / `setFeedback`, custom layout traps.
- `03-actions-and-behavior.md` — twelve actions, live variants, coalescing.
- `04-visual-spec.md` — palette, geometry, glyphs, PNG sizes.
- `05-live-state.md` — theme / Hyprland / audio / night / record sources and redraw rules.
- `06-install-migrate-verify.md` — manifest shape, install, migration, acceptance.

### Official protocol

- Elgato Stream Deck SDK, native plugin registration: [docs.elgato.com/streamdeck/sdk/references/websocket/plugin](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/).
- Elgato dials and touch-strip layouts: [docs.elgato.com/streamdeck/sdk/guides/dials](https://docs.elgato.com/streamdeck/sdk/guides/dials).
- Elgato layout schema (200×100 Encoder canvas, pixmap `value` may be a data URI): [docs.elgato.com/streamdeck/sdk/references/touch-strip-layout](https://docs.elgato.com/streamdeck/sdk/references/touch-strip-layout).
- Elgato plugin guidelines: at most ~10 programmatic touch-strip updates per second; `showAlert` for failed actions ([docs.elgato.com/guidelines/stream-deck/plugins](https://docs.elgato.com/guidelines/stream-deck/plugins)).
- OpenAction API (OpenDeck’s documented plugin protocol, Stream Deck–compatible): [openaction.amankhanna.me](https://openaction.amankhanna.me), states (`setImage`, `setFeedback`, `setFeedbackLayout`) at [openaction.amankhanna.me/api/serverbound/states.html](https://openaction.amankhanna.me/api/serverbound/states.html).

### OpenDeck host

- [nekename/OpenDeck](https://github.com/nekename/OpenDeck) and its [AGENTS.md](https://github.com/nekename/OpenDeck/blob/main/AGENTS.md): Tauri v2 host, plugins are child processes, WebSocket from port 57116, HTTP static files on port+2, `CodePathLin` / `CodePaths`, context validation so a plugin can only `setImage` its own instances.
- Installed Starter Pack plugin: `~/.config/opendeck/plugins/com.amansprojects.starterpack.sdPlugin/` (Rust binary `opendeck-starterpack`, same packaging convention).

### Reference plugin on this machine

- Installed bundle: `~/.config/opendeck/plugins/fr.jourdois.pipewire.sdPlugin/` (v0.4.0).
- Source: [sjourdois/opendeck-pipewire](https://github.com/sjourdois/opendeck-pipewire).
- Unused Node counterpart (do not follow): `com.sfgrimes.pipewire-audio.sdPlugin` (`CodePathLin: index.js`).

### Rust crates (evaluated)

- [`openaction` 2.7.0](https://crates.io/crates/openaction) — OpenDeck author’s SDK. Used by the PipeWire plugin and OpenDeck’s own Starter Pack.
- [`streamdeck-rs`](https://github.com/mdonoughe/streamdeck-rs) — older unofficial wire crate; no action registry.
- [`streamdeck-plugin`](https://crates.io/crates/streamdeck-plugin) — new (2026-09), 9 downloads at research time.
- [`streamdeck-lib`](https://github.com/VeeLume/streamdeck-lib) — 1 star, git-only.
- Native drawing: [`tiny-skia`](https://crates.io/crates/tiny-skia) (CPU 2D + PNG, no text) plus [`fontdue`](https://crates.io/crates/fontdue) or [`cosmic-text`](https://crates.io/crates/cosmic-text) for the TTF.
- Theme watch: [`notify`](https://crates.io/crates/notify) (Linux = inotify).
- Hyprland: official IPC is two Unix sockets ([wiki.hypr.land/IPC](https://wiki.hypr.land/IPC/)); [`hyprland-rs`](https://github.com/hyprland-community/hyprland-rs) exists but is not required if we speak the sockets and `hyprctl -j` ourselves.

This machine already has `rustc` / `cargo` 1.98.1.

---

## 3. How Stream Deck plugins actually work

### 3.1 Process, not a library

Elgato’s host (and OpenDeck) launches **one child process per plugin**. A compiled plugin’s `main` receives four flags ([Elgato registration](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/)):

| Flag | Meaning |
|---|---|
| `-port` | WebSocket port on localhost |
| `-pluginUUID` | registration id |
| `-registerEvent` | event name to send (`registerPlugin`) |
| `-info` | JSON about the app and devices (safe to ignore) |

Observed on this machine (`02-opendeck-protocol.md`):

```
-port 57116 -pluginUUID <uuid> -registerEvent registerPlugin -info <json>
```

The plugin connects to `ws://127.0.0.1:<port>` and immediately sends:

```json
{"event":"<registerEvent>","uuid":"<pluginUUID>"}
```

After that, traffic is JSON text frames. If the process exits, **nothing restarts it** until OpenDeck itself restarts. The binary must swallow errors and stay alive.

OpenDeck details that Elgato’s docs do not emphasise ([OpenDeck AGENTS.md](https://github.com/nekename/OpenDeck/blob/main/AGENTS.md), local `02`):

- Port search starts at **57116**; the HTTP asset server is **port + 2**.
- Messages queued before registration are replayed after `registerPlugin` (`PLUGIN_QUEUES`).
- Plugin stdout/stderr land in `~/.local/share/opendeck/logs/plugins/<uuid>.log`.
- A plugin may `setImage` only its own contexts. `setFeedback` is more permissive in OpenDeck; we must not rely on that.
- Profile JSON is rewritten from memory. Never edit it while OpenDeck is running.
- Every `setImage` stores a data URI on the instance and marks the profile dirty. Call it only when the picture changed.

### 3.2 Manifest

Stream Deck SDK format, plus OpenDeck extensions (`CodePathLin`, `CodePaths` target triples). Linux native binaries are executed directly; `.js` / `.mjs` / `.cjs` go through Node ≥ 20; `.html` goes through a hidden webview (`02`).

The working PipeWire plugin’s manifest is the packaging template:

```json
"CodePaths": {
  "x86_64-unknown-linux-gnu": "x86_64-unknown-linux-gnu/bin/opendeck-pipewire",
  "aarch64-unknown-linux-gnu": "aarch64-unknown-linux-gnu/bin/opendeck-pipewire"
},
"CodePathLin": "x86_64-unknown-linux-gnu/bin/opendeck-pipewire",
"OS": [{ "Platform": "linux" }]
```

OpenDeck snapshots each placed action into the profile. After a plugin update, **bump `Version`** or already-placed buttons keep the old action definition (icons, encoder layout, states). The PipeWire README is explicit about this; our install notes must be too.

### 3.3 Contexts and events

A placed instance is identified by a context string (`02`, OpenDeck `Context`):

```
<device>.<profile>.<controller>.<position>.<index>
sd-EL31L1A08599.Default.Keypad.0.0
sd-EL31L1A08599.Default.Encoder.2.0
```

The plugin must not assume fixed slots. The user can drag actions around. The live table is `context → action uuid`, filled from `willAppear` / `willDisappear`.

Events we must handle (Elgato + OpenAction `Action` trait, `openaction` 2.7):

| Host event | `openaction::Action` method | Our use |
|---|---|---|
| `willAppear` | `will_appear` | remember instance, draw immediately |
| `willDisappear` | `will_disappear` | forget instance |
| `keyDown` | `key_down` | fire the Omarchy command (ignore `keyUp`) |
| `dialRotate` | `dial_rotate(ticks, pressed)` | volume / workspace / theme |
| `dialDown` | `dial_down` | mute / menu / wallpaper |
| `touchTap` | `touch_tap(pos, hold)` | same as dial press for all four dial actions |
| everything else | default no-ops | tolerate, never crash |

Outgoing commands we will use (`Instance` in openaction 2.7):

- `set_image(Some(data_uri), Some(0))` — keypad, 144×144 PNG data URI, `target` 0 (hardware + software).
- `set_feedback({ "panel": data_uri })` — encoder touch strip.
- `show_alert()` — command missing or failed.
- `log` via `log` crate → stdout → OpenDeck plugin log.

We do **not** need a property inspector, `setSettings`, or `setTitle` for the live look. Titles painted into the PNG.

### 3.4 Encoder layouts — the easy way to get this wrong

A Stream Deck + touch strip is four independent **200×100** canvases, one per dial ([Elgato layouts](https://docs.elgato.com/streamdeck/sdk/references/touch-strip-layout)). Built-in layouts:

| Id | Shape | Why we will not use it |
|---|---|---|
| `$A0` | full-canvas pixmap **plus** a title item | title is the action name; empty pixmaps become a gray checkerboard (`02`) |
| `$B1` | title + 48×48 icon + value + bar | this is what the PipeWire plugin uses; it cannot draw our CRT frame, scanlines, or custom glyphs |
| `$A1`, `$B2`, `$C1`, `$X1` | other stock widgets | same problem |

The PipeWire plugin ships `layouts/volume.json`, a `$B1` clone with a larger title, and calls `set_feedback_layout("layouts/volume.json")` on `will_appear` so already-placed dials pick it up. That is the right *pattern* (runtime layout + `setFeedback` by item key) and the wrong *layout* for us.

Our layout must be a **single full-canvas pixmap with no default `value` and no title item** (`02`, `06`):

```json
{
  "$schema": "https://schemas.elgato.com/streamdeck/plugins/layout.json",
  "id": "vincent-panel",
  "items": [
    { "key": "panel", "type": "pixmap", "rect": [0, 0, 200, 100] }
  ]
}
```

Facts that bit the previous setup and the PipeWire plugin:

1. A pixmap `value` that points at a file is re-applied on every redraw and **overwrites** `setFeedback` (`02`). Do not set a default.
2. Paths that resolve outside the plugin folder are rejected (`Attempted to load image outside of base path`). The unused Starter Pack `layouts/strip-*.png` copies are an example of that trap.
3. `Encoder.background` is not painted on the strip (`02`).
4. Feedback lives in OpenDeck memory only. After a host restart the strip is blank until the plugin feeds it again (`02`).
5. Elgato accepts PNG data URIs, SVG strings, or plugin-local files for pixmap `value` ([layout pixmap](https://docs.elgato.com/streamdeck/sdk/references/touch-strip-layout)). Local `02` verified PNG. We will emit PNG only.
6. Until the first `setFeedback`, the slot is a gray checkerboard. Feed every encoder on `willAppear`.

---

## 4. The plugin we already run: `fr.jourdois.pipewire`

This is the native plugin currently on Encoder 0 (Output Volume) and Encoder 1 (Input Volume). It is the best local existence proof that a Rust OpenDeck plugin works on this machine.

### 4.1 What it is

- Plain Rust binary, **no Node**.
- Speaks OpenAction / Elgato WebSocket through **`openaction` 2.7**.
- Drives audio through the native **`pipewire` 0.10** crate (bindgen + libclang at build time). `wpctl` / `pactl` are not used at runtime.
- Tokio 1, `simplelog` to stdout, edition 2024.
- Packaged exactly like OpenDeck built-ins: `assets/` + `<triple>/bin/opendeck-pipewire`.
- GPL-3.0-or-later. `openaction` is MIT.

Installed binary (not stripped) confirms those crate versions: `openaction-2.7.0`, `tokio-1.53.1`, `pipewire-0.10.1`, `tokio-tungstenite-0.28.0`.

### 4.2 Module map (source of [sjourdois/opendeck-pipewire](https://github.com/sjourdois/opendeck-pipewire))

```
src/main.rs          register actions, start PipeWire thread, spawn refresh loop, run()
src/actions/*.rs     one Action impl per UUID; thin: event → command + redraw
src/command.rs       intents sent to the PipeWire thread (AdjustVolume, SetMute, …)
src/pw/              PipeWire main-loop on its own OS thread (not Send for Tokio)
src/refresh.rs       instance-id → settings table + refresh_all()
src/display.rs       keypad vs encoder dispatch; last-preview cache
src/render.rs        SVG key images + setFeedback JSON for $B1
src/color.rs, ui.rs  bar colours, custom icon, 100 % cap
```

`main` does four things, in order:

1. Init `TermLogger` on stdout (OpenDeck captures it).
2. Start the PipeWire backend. **On failure it still calls `run()`** so OpenDeck does not treat the plugin as crashed.
3. `register_action(...)` for each action, injecting shared handles.
4. `tokio::spawn` a loop: wait on a `watch` channel, coalesce, `refresh_all()`, then `run(args)`.

That “stay connected even if a backend dies” rule is one we should copy.

### 4.3 Patterns we should steal

**Action objects hold shared handles, not process-global statics.**

```rust
pub struct VolumeAction {
    pub pw: PwHandle,
    pub refresher: Refresher,
}
```

**`will_appear` registers the instance; `will_disappear` forgets it.** `openaction` does not expose stored settings to a background task, so the PipeWire plugin keeps a `Refresher`: `Arc<Mutex<HashMap<instance_id, Settings>>>`. Out-of-band redraws then call `visible_instances(uuid)` (openaction 2.7) and look up settings.

We have no property-inspector settings, but we still need the same table: `instance_id → Arc<Instance>` plus enough snapshot state to redraw. `visible_instances` is the official enumeration; keep a side table only for last-sent image hashes and any optimistic audio values.

**One display path per surface, shared by events and watchers.** `display::volume(...)` is used from `key_down`, `dial_rotate`, and `refresh_all`. Keypad vs encoder is an internal branch (`instance.controller == "Encoder"`).

**Do not resend an unchanged picture.** `display.rs` caches the last encoder preview per instance because every `setImage` refreshes the UI and dirties the profile. We must do the same for both `setImage` and `setFeedback` (hash of PNG bytes is enough).

**Optimistic UI, then confirm.** Mute flips the bar immediately; the PipeWire event reconciles. Volume ticks should do the same (`05`).

**Tick math lives in the action, I/O in a backend.** `dial_rotate` receives `ticks: i16` and applies `ticks * step` once. Fast spins become one command, not one `wpctl` per detent.

**UUID prefix in one place.**

```rust
macro_rules! action_uuid {
    ($suffix:literal) => { concat!("fr.jourdois.pipewire.", $suffix) };
}
```

### 4.4 Patterns we must not copy

| PipeWire plugin | Why not for Vincent Deck |
|---|---|
| `$B1` / `layouts/volume.json` with title, icon, value, bar | Cannot paint the CRT panel. Custom one-pixmap layout. |
| SVG `data:image/svg+xml;base64,...` key images | Spec requires PNG, exact ImageMagick geometry, Terminess at discrete pixel sizes. OpenDeck accepts SVG; we still emit PNG (`02`, `04`). |
| Native `pipewire` crate | Correct for a dedicated audio plugin; for us it adds bindgen, libclang, and a non-Send thread. Spec already chose `wpctl` + `pactl subscribe` (`05`). Acceptance test 12 is “`wpctl` missing from PATH”. |
| Property inspectors, per-action colours, custom icons | Out of scope. Palette comes from Omarchy. |
| `set_title` / OpenDeck preview-title games | We paint text into the PNG. Leave titles empty. |
| Volume step default of 5 % | Spec is **1 % per tick**, clamp 100 % (`03`). |
| Cubic / perceptual scale and 150 % boost | Spec is linear percent 0–100 via `wpctl -l 1.0`. |

### 4.5 The unused Node plugin

`com.sfgrimes.pipewire-audio.sdPlugin` is a Node `index.js` + `ws` plugin with the same `$B1` encoder idea. It is installed and unused. It is the old shape: ship Node, spawn `wpctl` from JS, live in `node_modules`. OpenDeck can run it (`02`: `.js` → Node ≥ 20), but it is not the model for this repo. The native PipeWire plugin already replaced that approach for audio; we replace the rest the same way.

---

## 5. Rust ecosystem: what to depend on

### 5.1 Plugin SDK — choose `openaction` 2.7

| Crate | Role | Verdict |
|---|---|---|
| **`openaction` 2.7.0** | Official OpenAction Rust SDK (nekename). Parses argv, connects, registers, dispatches `Action` methods, `visible_instances`, `Instance::{set_image,set_feedback,show_alert}`. ~30k downloads. | **Use this.** Same crate, same version as the plugin that already works here. |
| `streamdeck-rs` | Arg parse + raw socket. Last real activity years ago; examples still show old `tokio::run`. | Skip. We would reimplement the action registry. |
| `streamdeck-plugin` / `streamdeck-plugin-protocol` | New high-DX framework (2026-09). 9 downloads. | Skip. Unproven next to OpenDeck. |
| `streamdeck-lib` | Git dependency, 1 star. | Skip. |

Elgato’s current docs *discourage* native plugins and push Node + `@elgato/streamdeck`. That advice is for the Elgato Marketplace and Windows/macOS first-party host. OpenDeck’s own plugins, the Starter Pack, and the PipeWire plugin are native Rust. On this machine, native is the supported path.

`openaction::run` blocks until the socket drops. `register_action` must happen first. Actions are `Send + Sync + 'static`; handlers are `async_trait`.

### 5.2 Drawing — `tiny-skia` + a TTF rasteriser, not ImageMagick

Requirements from `04`: 144×144 and 200×100 sRGB PNG, exact geometry, Terminess Nerd Font Mono **by file path**, crisp only at 14/16/18/20/24/28/32, anti-aliasing on, well under 100 ms, cache by inputs.

| Approach | Fit |
|---|---|
| Keep spawning `magick` (current `deck-icons`) | Matches look, but a process per tile, PATH/fontconfig risk inside the plugin, too slow for 12-tile theme bursts and 1 Hz record timer. |
| SVG like PipeWire | Easy bars; cannot hit pixel-font crispness or scanline geometry without becoming a second renderer. Spec says PNG. |
| `tiny-skia` + `fontdue` | Pure Rust, PNG encode built in, load TTF from `/usr/share/fonts/TTF/TerminessNerdFontMono-Bold.ttf`. No fontconfig. Enough for gradients, rounded-rect strokes, 1 px scanlines, circles, bars, pips, swatches. |
| `cosmic-text` + `tiny-skia` | Better shaping/ellipsis; heavier. Useful if theme names overflow; we can add it later. First cut: measure with `fontdue` and truncate. |

Proposal: **`tiny-skia` + `fontdue` + `png`**. Port `deck-icons` layer-for-layer (gradient, frame, scanlines, glyph, label, tag, corner dot). Preserve legacy fixtures for visual comparison. Require exact dimensions/layout and perceptual fidelity on the device, not identical PNG hashes or pixel buffers across different rasterizers. Validate one key and one panel before committing to the rendering libraries.

Do not depend on the font being “installed by name”. Open the file. If it is missing, log and draw without glyphs rather than panic.

### 5.3 Live state — thin adapters, not extra frameworks

| Domain | Production adapter | Why |
|---|---|---|
| Theme | `notify` on `~/.local/state/omarchy/current/theme.name` + optional hook in `~/.config/omarchy/hooks/theme-set.d/` that touches that file or a FIFO | Spec wants event-driven + debounce 300–500 ms; `omarchy-theme-color --all` is racy mid-switch (`05`). |
| Hyprland | Unix stream to `$XDG_RUNTIME_DIR/hypr/<sig>/.socket2.sock`; re-query with `hyprctl -j` | Official IPC. Rediscover newest live socket if `HYPRLAND_INSTANCE_SIGNATURE` is missing. `hyprland-rs` is optional sugar; a 50-line reader is enough and easier to fake in tests. |
| Audio | `pactl subscribe` + `wpctl get-volume` / `set-volume` / `set-mute` | Spec. Fallback poll 500 ms. Coalesce ticks. Clamp `-l 1.0`. |
| Night | poll `omarchy-toggle-nightlight --status` every ~2 s; extra samples after press | No event source (`05`). |
| Record | `pgrep -f "^gpu-screen-recorder"` + `/proc/<pid>` start time; 1 s while active, 2 s idle | Spec. |

Do **not** take the `pipewire` crate. Two adapters (CLI vs native PipeWire) would be a real seam if we needed both; we do not. One adapter (process + parse) plus an in-memory fake for tests.

### 5.4 Process spawn

Omarchy launchers and `deck-workspace` / `deck-theme-cycle` must start **detached**: new session, ignore output, never block the event loop, reap so we create no zombies (`03`, `06`). Tokio `Command` with `pre_exec(setsid)`, stdin/stdout/stderr to null, then `tokio::spawn` a waiter. Prepend to `PATH`: `/usr/share/omarchy/bin` and `~/.local/bin`. Inherit Wayland/Hyprland env; if `HYPRLAND_INSTANCE_SIGNATURE` is missing, rediscover before spawning (`01`, `05`).

`deck-theme-cycle` already serialises with `flock`. After migration, remove its `deck-icons &` line (`06`). Until then the generator racing the plugin is harmless if we own every context.

---

## 6. Proposed architecture

### 6.1 Shape

One crate, one binary, one `.sdPlugin` folder. Twelve `openaction::Action` types. Shared deep modules behind them.

```
                 OpenDeck (host)
                      │  WebSocket JSON
                      ▼
                 openaction::run
                      │
        ┌─────────────┼──────────────┐
        ▼             ▼              ▼
   Action impls    Watchers      Surface
   (thin)          (theme,       (setImage /
                    hypr,         setFeedback
                    audio,        + last-hash)
                    night,
                    record)
        │             │              ▲
        ▼             ▼              │
      Cmd           Snapshots ──► Render
      (detached     (Theme,         (tiny-skia
       spawn)        Hypr,           PNG cache)
                     Audio,
                     Night,
                     Rec)
```

Vocabulary (from this repo’s codebase-design skill):

- **Module**: a thing with an interface and an implementation. Scale-agnostic.
- **Interface**: everything a caller must know (types, invariants, error modes, cost).
- **Seam**: where that interface lives.
- **Adapter**: a concrete satisfier of a seam.
- **Depth**: lots of behaviour behind a small interface.

Actions stay **shallow on purpose**: they translate a deck event into one call on a deep module. Depth lives in Theme, Render, Surface, Cmd, and the snapshot watchers.

### 6.2 Plugin identity

| Field | Value |
|---|---|
| Folder / plugin UUID | `dev.vincent.deck` |
| Installed path | `~/.config/opendeck/plugins/dev.vincent.deck.sdPlugin/` |
| Binary | `x86_64-unknown-linux-gnu/bin/vincent-deck` |
| Category | `Vincent Deck` |
| Version | `0.1.0` (bump on every install that changes action definitions) |

Action UUIDs:

```
dev.vincent.deck.terminal
dev.vincent.deck.browser
dev.vincent.deck.screenshot
dev.vincent.deck.record
dev.vincent.deck.agent
dev.vincent.deck.clipboard
dev.vincent.deck.night
dev.vincent.deck.lock
dev.vincent.deck.volume
dev.vincent.deck.mic
dev.vincent.deck.workspace
dev.vincent.deck.theme
```

Keys: `Controllers: ["Keypad"]`. Dials: `Controllers: ["Encoder"]`. Support rearrangement and duplicate instances within the original controller type; cross-controller placement is excluded. Encoder block points at `layouts/panel.json`. No property inspector. `SupportedInMultiActions: false`.

### 6.3 Source layout

```
vincent-deck/                         # this repo, after implementation
├── RESEARCH.md                       # this file
├── 01-… 06-…                         # product contract
├── Cargo.toml
├── rustfmt.toml
├── assets/
│   ├── manifest.json
│   ├── layouts/panel.json
│   └── icons/                        # tiny static PNGs for the OpenDeck sidebar only
├── scripts/install.sh                # cargo build --release && rsync into ~/.config/opendeck/plugins/
└── src/
    ├── main.rs
    ├── actions/
    │   ├── mod.rs                    # action_uuid! macro
    │   ├── launch.rs                 # the six fire-and-forget keys
    │   ├── night.rs
    │   ├── record.rs
    │   ├── volume.rs
    │   ├── mic.rs
    │   ├── workspace.rs
    │   └── theme.rs
    ├── cmd.rs                        # detached spawn + PATH
    ├── theme.rs                      # palette, watch
    ├── hypr.rs                       # socket2 + hyprctl snapshots
    ├── audio.rs                      # wpctl / pactl
    ├── night.rs                      # --status poll
    ├── record.rs                     # pgrep + elapsed
    ├── render.rs                     # PNG tiles and panels
    ├── surface.rs                    # last-hash setImage / setFeedback
    └── refresh.rs                    # visible-instance registry + coalesced redraw
```

`assets/` is copied as-is into the `.sdPlugin` folder, same as the PipeWire plugin. The binary is the only build artifact.

### 6.4 Deep modules

#### Theme

**Interface.** `Theme` exposes:

- `fn current(&self) -> Palette` — theme colours, preserving monochrome accents.
- `fn name(&self) -> ThemeName` — machine name + prettified name + `index/count`.
- `fn subscribe(&self) -> watch::Receiver<ThemeEpoch>` — bumps when a redraw-everything is required.

**Implementation.** Read `theme.name`; run `omarchy-theme-color --all`; parse `name\t#rrggbb`; preserve all theme accents, including grayscale (`04`). Watch the file with `notify`. Debounce ≥ 400 ms. If the name changed but the palette did not, schedule one more read ~1 s later (`05`). A hook in `theme-set.d` may bump the same epoch; it is an extra trigger, not a second source of truth.

**Cache.** Palette change clears the render cache and requests a full redraw. Fast theme-dial spins keep **only the latest** pending full redraw (watch channel semantics).

Unreadable palettes retain the last valid colors with stale theme status. A cold
start without a valid palette uses neutral grayscale and theme unavailable.
Valid monochrome colors are preserved rather than treated as missing data.

#### Render

**Interface.** Pure function of inputs → PNG bytes:

```text
render_key(tile: KeyTile) -> Png
render_panel(panel: Panel) -> Png
```

`KeyTile` / `Panel` are owned value types (action identity, palette, live extras: night on, formatted recording elapsed text, volume percent + mute, workspace pips, theme swatches). Same palette + same visible state ⇒ same bytes. Use a bounded cache keyed by those inputs, never an unbounded history of timestamps or window titles.

**Implementation (provisional libraries).** `tiny-skia` pixmap; `fontdue` with the Terminess file; layers in `04` order. Night-on and record-active are variants of `KeyTile`, not a second renderer. Dial actions render panels only.

**Tests (the interface is the test surface).** Approved renderer fixtures for one dark, one light, one monochrome theme, idle keys, muted volume, night-on, record `01:23`, and stale state. Check geometry against the spec independently. Legacy fixtures establish perceptual fidelity; exact cross-renderer hashes are not an acceptance criterion. Physical-device appearance requires user approval.

#### Surface

**Interface.**

```text
fn push_key(instance: &Instance, png: &Png) -> Result<()>
fn push_panel(instance: &Instance, png: &Png) -> Result<()>
```

**Implementation.** Base64 data URI; skip the WebSocket send if the hash matches the last one sent for that `instance_id` (PipeWire `PREVIEW` cache, applied to both keys and panels). Encoder → `set_feedback({ "panel": uri })`. Keypad → `set_image(uri, state 0)`. Forget the hash on `will_disappear`; invalidate last-sent state on reconnect so unchanged images are restored too.

This is the module that protects the profile file from being rewritten twelve times a second.

#### Cmd

**Interface (proposal).** Start a detached command without blocking the event
loop, returning a completion handle or equivalent asynchronous notification.
Night/Record busy gates and Workspace coalescing require completion information;
a fire-and-forget `Result<()>` alone is insufficient. Log failures and show the
agreed brief failure indication on the originating context.

**Implementation.** Tokio `Command`, `setsid`, stdio null, PATH prefix, background `wait()`. Missing binary is an error, not a panic.

#### Snapshots (Audio, Hypr, Night, Record)

Each is a deep module with the same shape: `snapshot() -> T` + `subscribe() -> watch::Receiver<()>` (or a ticker). Actions and `refresh` only read snapshots. Writes (volume ticks, mute) go through the same module so optimistic updates can patch the snapshot before the next event.

Audio coalesce: a mutex’d `pending_ticks: i16` per default sink/source; a single in-flight `wpctl`; further ticks add into `pending` (`05`).

Hypr: on any listed socket2 event, re-run `hyprctl workspaces -j`, `activeworkspace -j`, `activewindow -j`. Occupied = `windows > 0` on any monitor. Current = focused monitor’s workspace. Socket gone → backoff reconnect, never exit.

Record elapsed uses process start time so a plugin restart mid-recording stays correct (`05`).

### 6.5 Action modules (thin)

Each action is an `openaction::Action` with `Settings = ()` (or an empty serde struct so unknown JSON does not fail). They share a `App` handle:

```text
App { theme, render, surface, cmd, audio, hypr, night, record, refresh }
```

Typical keypad (`terminal`, `browser`, …):

- `will_appear` → render idle tile → `surface.push_key`
- `key_down` → `cmd.spawn([...])`; on error `show_alert`
- If `show_alert` is unsupported, render a brief failure indication instead;
  command failures always log details. No extra desktop notification is required.
- theme epoch → `refresh` redraws this instance

Night / Record add their snapshot to `KeyTile`. Both use stable filled indicators and filled/inverted label areas; Night explicitly indicates ON. Record displays total minutes/seconds through `99:59`, then compact hours/minutes. Poll at 1 Hz while recording but redraw only when visible text changes (`05`).

Each toggle action has a shared busy gate: show pending and drop repeat presses
until its detached command completes, then requery state. Duplicates share the
gate. Pending is not confirmed active state. Input during unavailable state is
attempted once when possible, never queued for recovery.

Volume / Mic:

- `dial_rotate` → `audio.nudge(sink|source, ticks)` (1 % × ticks, clamp 0–100) → optimistic panel
- `dial_down` / `touch_tap` → `audio.toggle_mute` → optimistic panel
- audio watch → `refresh` those UUIDs only

Workspace / Theme:

- `dial_rotate` → `cmd.spawn(["deck-workspace"|"deck-theme-cycle", ticks])` (sign of ticks; one spawn per coalesced event)
- Workspace keeps the latest pending direction behind one in-flight command;
  Theme drops busy input entirely. Ignore zero ticks for both.
- `dial_down` / `touch_tap` → `omarchy-menu` / `omarchy-theme-bg-next`
- hypr watch → workspace panel only
- theme watch → everything, including the theme panel name and swatches

`refresh.rs` is the PipeWire `refresh_all` idea, split by trigger:

| Trigger | Redraw |
|---|---|
| `willAppear`, reconnect | that instance |
| theme epoch | all visible instances |
| hypr event | workspace instances |
| audio event | volume + mic instances |
| night poll change | night instances |
| record start/stop / 1 s tick | record instances |

Use `visible_instances(UUID)` so rearranging slots cannot strand a stale context.

### 6.6 Runtime in `main`

```text
TermLogger → stdout
build App (theme watch, hypr task, audio task, night ticker, record ticker)
register 12 actions
tokio::spawn refresh router (select! on all watch receivers, coalesce 50–100 ms)
run(env::args())
```

If Hyprland or PipeWire is down at start, still `run()`. Draw what we can and visibly mark affected state unavailable/stale; never substitute confirmed muted or empty for an unreadable state. Any retained last-known value must be dimmed and qualified. Retry backends with backoff and restore normal display after fresh successful reads. A dead backend must not kill the plugin; a plugin-process crash itself may require an OpenDeck restart.

Idle CPU: no busy loops. `notify` + socket2 + `pactl subscribe` + two interval timers. Enforce an explicit at-most-10-Hz per-panel send policy: 50 ms coalescing does not itself enforce this limit, and trailing debounce must not starve display updates during sustained events. Cache budgets, query timeouts, retry limits, and watch recovery details are delegated to the implementer with measurements and documentation. Interactive commands require distinct lifecycle handling from timed state queries.

### 6.7 Manifest and layout (install bundle)

```
dev.vincent.deck.sdPlugin/
├── manifest.json
├── layouts/panel.json          # no default pixmap value
├── icons/                      # sidebar defaults only
│   ├── plugin.png
│   ├── terminal.png
│   └── …
└── x86_64-unknown-linux-gnu/bin/vincent-deck
```

`scripts/install.sh`:

1. `cargo build --release`
2. rsync `assets/` + binary into `~/.config/opendeck/plugins/dev.vincent.deck.sdPlugin/`
3. print the OpenDeck restart line: `pkill -x opendeck; setsid nohup opendeck >/dev/null 2>&1 &`

Migrate only after validation on a test profile (`06`). The agreed path is a scripted twelve-slot profile rewrite with OpenDeck stopped and the complete legacy backup defined in `06`, preserving rollback after cleanup. Provide one command to restore the profile, assets, and affected scripts together.

---

## 7. Event and data flow (happy path)

```
user spins Volume dial
  → OpenDeck outbound dialRotate { ticks: 3 }
  → VolumeAction::dial_rotate
  → audio.nudge(+3%)                # one wpctl set-volume 3%+ -l 1.0
  → snapshot patched optimistically
  → render_panel(volume, palette, 45%, unmuted)
  → surface.push_panel (skip if hash unchanged)

pavucontrol moves the same sink
  → pactl subscribe "change on sink"
  → audio refreshes snapshot
  → refresh router wakes
  → visible_instances(volume) + visible_instances(mic)
  → push only if percent/mute/device name changed

omarchy-theme-set from a terminal
  → theme.name mtime / hook
  → debounce 400 ms
  → re-read palette; preserve grayscale if mono
  → clear render cache
  → latest-only full redraw of every visible instance
```

---

## 8. Decisions

| Decision | Choice | Why |
|---|---|---|
| Language | Rust 2024, rustc 1.98 already on the machine | Matches OpenDeck and the working native plugin; no new runtime inside the plugin process. |
| SDK | `openaction` 2.7 | Same crate the host author and the PipeWire plugin use. |
| Images | PNG data URIs, 144×144 / 200×100 | Verified path on this host; profile stores them; webview will not cache a file path. |
| Encoder layout | Custom one-slot pixmap `panel` | Only way to own the CRT look; `$B1` is what we are leaving. |
| Audio backend | `wpctl` + `pactl subscribe` | Spec + acceptance test 12; avoids bindgen. Native PipeWire stays in the installed plugin until we migrate off it. |
| Hyprland | socket2 + `hyprctl -j` | Official IPC; no extra crate. |
| Theme watch | `notify` + debounce + optional hook | Spec; race-safe. |
| Draw crate (provisional) | `tiny-skia` + `fontdue` | Validate perceptual fidelity on one key and one panel before committing. |
| Settings / PI | None | All behaviour is fixed; live state is the world, not JSON. |
| License | MIT or Apache-2.0 for *our* code | We are not forking the GPL PipeWire plugin; we are writing a new plugin that only copies ideas. |

Rejected:

- **Node plugin** (`@elgato/streamdeck` or the unused `com.sfgrimes` style). Extra runtime, `node_modules` in `~/.config`, worse idle profile.
- **Shell plugin** (shebang script). Allowed by OpenDeck, too easy to block the event loop, no good PNG cache.
- **Driving other plugins’ contexts** via `setFeedback`. Works in OpenDeck, forbidden by `02`, and we are migrating off them.
- **Writing icon files for the Starter Pack layouts.** That is the current broken path (`deck-icons` copies into another plugin’s folder).

---

## 9. Risks and VERIFY items

### Local script inspection during the design interview (2026-10-03)

- `~/.local/bin/deck-workspace` delegates navigation to Hyprland `e+1` / `e-1`
  and ignores magnitude. Exact endpoint/monitor wrapping is not established by
  that script. Vincent Deck will ignore zero ticks and preserve sign-based input,
  with one in-flight request and only the latest pending direction.
- `~/.local/bin/deck-theme-cycle` uses nonblocking `flock`: busy requests are
  dropped, not queued. Accepted requests wrap modulo the theme list.
- `/usr/share/omarchy/bin/omarchy-toggle-nightlight` has a retry loop but no
  serialization. Concurrent invocations may read the same old state or compete.
- `/usr/share/omarchy/bin/omarchy-capture-screenrecording` has no input lock.
  Region selection occurs before a recorder process exists, so another start
  invocation can open another selector. Selection cancellation exits without a
  recorder; stopping an active recording sends SIGINT and waits for saving.
- `/usr/share/omarchy/bin/omarchy-theme-set` stages a new theme and replaces the
  current theme directory through deletion and rename, so a watch attached only
  to a replaced palette inode is insufficient. Its post-retint `theme-set` hook
  is available in normal desktop operation; the critical-section lock is released
  before the hook, so always read current state rather than trust its argument.

These are factual inputs to pending interaction decisions, not authorization to
change behavior beyond the agreed numbered specs.

From `02` / `04` / `06`, still unproven until implementation:

| Item | Risk | Mitigation |
|---|---|---|
| Queued `willAppear` before register | Missed first draw | Log startup; `openaction` should replay; if not, draw again on a 200 ms timer once. |
| `showAlert` / `logMessage` on OpenDeck 2.14 | May be no-ops | Try; logging to stdout is the real diagnostic. |
| Muted-speaker / muted-mic glyphs U+F026 / U+F131 | May be missing from Terminess | Probe the TTF in a unit test; fall back to U+F6A9 or a drawn slash. |
| `fontdue` vs ImageMagick hinting | Perceptual appearance differs | Compare preserved references and verify on device; minor rasterization differences are acceptable. |
| Light and mono themes | State distinctions may be unclear | Preserve grayscale accents; use text, glyphs, fill, and shape. Verify Catppuccin Latte and Vantablack on device. |
| Profile rewrite volume | Fast record timer + theme spin | Hash gate on Surface; latest-only theme redraw. |
| Plugin kill | Stays dead until OpenDeck restart | Document it (`06` #11). Do not add a watchdog that fights OpenDeck. |
| `deck-theme-cycle` still calls `deck-icons` | Extra disk writes during migration | Remove after acceptance (`06`). |

---

## 10. Suggested implementation order

1. **Skeleton.** `Cargo.toml`, `openaction` hello-action, `assets/manifest.json` with one keypad action, `scripts/install.sh`, restart OpenDeck, confirm plugin log and `willAppear`.
2. **Surface + Render.** Idle Terminal tile matching `deck-icons` on one dark theme. Golden test. Then the other seven idle keys.
3. **Custom encoder layout.** Blank 200×100 panel on `willAppear` (kills the checkerboard). Then Volume panel from a fake snapshot.
4. **Cmd.** Detached spawn + PATH. Wire the six launch keys on `keyDown`.
5. **Theme module.** `notify` + strict theme palette + full redraw. Acceptance 8–9 become possible.
6. **Audio.** `wpctl` / `pactl subscribe`, 1 % ticks, mute, optimistic panel, unavailable state. Validate on a test profile before live migration in step 10.
7. **Hypr + workspace panel.** socket2, pips, active window.
8. **Theme dial.** Reuse `deck-theme-cycle`; panel name + swatches + `n/N`.
9. **Night + Record** live keys.
10. **Profile migration** (`06`), acceptance list, then cleanup (only after the user agrees).

Do not migrate the live profile until step 10. Keep Starter Pack and PipeWire installed unused, as `01` already says.

---

## 11. What “done” looks like

The installed folder is self-contained. OpenDeck starts one Rust process. That process:

1. Registers twelve actions.
2. Paints every visible instance on `willAppear`.
3. Runs Omarchy commands without blocking.
4. Follows theme, Hyprland, PipeWire, night light, and `gpu-screen-recorder` without a busy loop.
5. Never exits on a missing tool.
6. Leaves the old plugins installed until the user deletes them.

The look is the current CRT tiles. The architecture is the PipeWire plugin’s process model and refresh loop, with a custom pixmap layout and a PNG renderer instead of `$B1` + SVG + libpipewire.

---

## 12. Citation index

- Local specs `01-environment.md` … `06-install-migrate-verify.md`.
- `~/.local/bin/deck-icons`, `deck-workspace`, `deck-theme-cycle` (current behaviour).
- `~/.config/opendeck/plugins/fr.jourdois.pipewire.sdPlugin/manifest.json` and `layouts/volume.json`.
- [sjourdois/opendeck-pipewire](https://github.com/sjourdois/opendeck-pipewire) `README.md`, `Cargo.toml`, `src/main.rs`, `src/refresh.rs`, `src/display.rs`, `src/render.rs`, `src/actions/volume.rs`.
- [nekename/OpenDeck AGENTS.md](https://github.com/nekename/OpenDeck/blob/main/AGENTS.md).
- [Elgato plugin WebSocket](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/), [dials](https://docs.elgato.com/streamdeck/sdk/guides/dials), [touch-strip layout](https://docs.elgato.com/streamdeck/sdk/references/touch-strip-layout).
- [OpenAction states](https://openaction.amankhanna.me/api/serverbound/states.html); [openaction 2.7 docs](https://docs.rs/openaction/2.7.0/openaction/).
- [Hyprland IPC](https://wiki.hypr.land/IPC/).
- crates.io: `openaction` 2.7.0, `tiny-skia`, `fontdue`, `notify`.
