# Implementation notes

The Default profile was migrated on 2026-10-03. The user confirmed basic operation
and approved the appearance. Release builds have since been installed and
registered after OpenDeck restarts. Full hardware and performance acceptance is
still incomplete.

The Agent control uses `dev.vincent.deck.agent` and `icons/agent.png` throughout
the source and generated manifest. It toggles Omarchy's default agent scratchpad;
it does not launch a particular agent product. Existing profiles with the old
action ID need their placed definitions updated while OpenDeck is stopped.

Lock follows Omarchy shell session-lock status every 500 ms. Confirmed lock uses
the Night-like filled/inverted treatment with `LOCKED`; pending acquisition shows
`WAIT`.

## Build and install

Rust 2024, OpenAction 2.7.0, Tokio, tiny-skia and fontdue. The renderer embeds
`assets/fonts/TerminessNerdFontMono-Bold.ttf` using `include_bytes!`, including
muted glyphs. It parses the embedded bytes without a filesystem/fontconfig lookup.
Font provenance and SIL OFL 1.1 notices are in `assets/fonts/`; installation copies
them into the bundle along with the assets. The binary needs no external font.
Python 3 is used only by installation/migration tooling; desktop commands remain
runtime dependencies.

```sh
python scripts/manage.py install
```

Installed bundle: `~/.config/opendeck/plugins/dev.vincent.deck.sdPlugin/`.
Build bundle: `dist/dev.vincent.deck.sdPlugin/`. Source: `src/`.
Restart OpenDeck to discover the bundle. Existing placed action definitions
require a manifest version bump when their definitions change.

See [docs/install.md](docs/install.md) for profile creation, migration, backups,
rollback, and hardware acceptance checks.

## Source layout

| File | Responsibility |
|---|---|
| `src/main.rs` | Sample rendering and the host reconnect loop |
| `src/actions.rs` | OpenAction registration and controller event routing |
| `src/app.rs` | Shared state, polling, rendering, input queues/gates, event streams |
| `src/display.rs` | Latest-only images, fair writer, stall status, bounded expiring host commands |
| `src/audio/mod.rs` | Native audio thread, command channel, latest-state notifications, reconnect |
| `src/audio/native.c` | Small WirePlumber 0.5 library bridge, mixer/default-node APIs |
| `src/state.rs` | Desktop queries, parsing, and snapshot accessors |
| `src/metrics.rs` | Read-only Linux sampling, counter deltas, bounded graph histories |
| `src/power.rs` | Available power-profile parsing and directional cycling |
| `src/process.rs` | Command environment, sessions, query timeout/output limits |
| `src/render.rs` | Action styles, palettes, elapsed text, font rasterization, PNG drawing |
| `scripts/manage.py` | Build/install, manifest generation, profile migration, rollback |

The application refactor separates polling and theme checks, refresh operations,
render passes, audio batches, busy gates, command execution, and event watchers.
`Surface` owns visible per-context flags and a generation-tagged delivery token; `RenderCache`
owns the bounded image cache. Snapshot accessors centralize field selection.
Content is still a common struct containing fields for all actions; action policy
dispatch remains in several matches.

## System statistics

Four additive Encoder-only actions (`cpu`, `memory`, `disk`, `network`) are
registered alongside the original twelve. One shared one-second sampler reads
`/proc/stat`, `meminfo`, `diskstats`, and `net/dev`; `/sys/block/*/device` and
`/sys/class/net/*/device` identify physical devices. No monitoring processes are
launched for telemetry. A skipped-tick interval avoids catch-up bursts. Sampling continues while
the plugin runs so a newly shown stats panel has recent history.

CPU/memory use a fixed 0–100% graph scale; disk/network use a shared rolling peak
scale for both series. Histories contain at most 60 values per series and are
part of visible render-cache keys. Source failures retain dimmed history and
reset counter baselines before recovery. Read/write and download/upload are
distinguished by solid accent and dashed foreground traces even in monochrome.

`stats-profile` copies Default's keys and custom images into a separate System
Stats profile, assigning only its copied dial slots. It never edits Default and
rejects overwriting existing stats profiles. `--name performance` names the copied
profile performance. Default filename casing is detected for profile commands and
legacy archives. The manifest version is now `0.2.4`.

CPU's footer independently tracks the active power profile every two seconds.
Rotation reads the available profiles and selects one signed step with
`omarchy-powerprofiles-set autodetect <profile>`, then confirms the active mode.
A shared lock serializes polling and writes; an action-wide gate drops repeat
rotations while busy. Missing profile information shows `POWER?` without hiding
valid utilization. CPU click/tap and Memory/Disk input remain inert.

Network screen-tap asynchronously summons Omarchy's `omarchy.speedtest` overlay.
The shell owns its download/upload phases and cancellation. A gate prevents
overlapping launch commands; network rotation is inert. Launch failures render
`ERROR` on the originating panel.

Theme and Network dial-down events now send OpenDeck's native `switchProfile`
message for the originating device: Theme targets `performance`, Network targets
`default`. Target profile files must exist. Screen taps are routed independently
to the original wallpaper/speed-test actions; dial-up does not switch a second
time. Other dial behavior is unchanged. The host harness verifies both switch
messages and that clicking does not additionally run a tap command.

Real-host testing revealed stock OpenDeck 2.14.0 silently rejects that message
from Vincent Deck. The owner-approved host allowlist patch is documented in
[docs/opendeck-host-patch.md](docs/opendeck-host-patch.md). The opt-in live test
reproduced the failed selection change on stock OpenDeck and passed in both
directions on the patched build. Mock-host message assertions alone missed this.

## Native audio

Audio uses one persistent PipeWire connection through WirePlumber 0.5's native
`default-nodes-api` and `mixer-api` modules. A dedicated GLib thread owns all native
objects. The small C bridge keeps GObject signals and native ownership out of the
Rust application; Cargo compiles it and links the system library. No `wpctl` or
`pactl` process is launched for audio, and audio is no longer polled.

The mixer uses the same cubic scale and hardware Route/software Props handling
as `wpctl`. Rotations still apply whole 1% ticks, clamped to 0–100%; observation
and mute retain externally set above-cap levels. Writes are serialized on the
audio thread, with native synchronization before the next write. Default changes,
node removal, volume/mute changes, and disconnects publish latest-state updates
through a deduplicated Tokio watch channel. Missing devices retain dimmed last-known
display values. Failed inputs are consumed; reconnect rediscovers the audio session
without replaying them.

Volume dial-click cycles output sinks; Mic dial-click cycles input sources, in stable
node-name order with wrapping. Screen taps still toggle mute, and rotations adjust
the effective default. Selection writes the configured default through native
WirePlumber policy and waits up to two seconds for the effective default to confirm.
A lone device is a no-op; absent devices or unconfirmed selection show `ERROR`.
Clicks do not change volume or mute on either device. Stream/monitor nodes are excluded
by exact sink/source media class. A small 11 px nickname/description line identifies
the selected device above the percentage without covering the icon/bar/status.
Device strings are bounded to 255 bytes and control characters are sanitized.

## Runtime limits

- State-query timeout: 2 seconds; captured query stdout capped at 256 KiB.
  Interactive launchers/selectors have no timeout. Commands run in separate
  sessions and children are waited asynchronously.
- Cache: at most 64 rendered data URIs; per-context last-sent image is bounded by
  visible instances. Window/theme input strings are bounded. Recording cache inputs
  contain formatted visible elapsed time only.
- Display send limit: 100 ms minimum per context (at most 10 Hz). Rendering copies
  descriptors under short-lived app locks and never awaits socket I/O. One writer
  owns one in-flight send; each visible generation retains at most one newest
  pending image and one last-sent image. A fair dirty queue prevents graph starvation.
  After two seconds a send is marked stalled but its future remains intact; no
  replacement writes start until it completes. Only ending that SDK session cancels
  it. Kernel/WebSocket buffers may still contain older accepted images.
- Host profile commands: at most eight queued, prioritized between image writes,
  expire after two seconds before starting. Stall/offline rejects new commands;
  a stalled in-flight command reports uncertain delivery and is never replayed.
  Surface generation changes and disconnects discard queued commands. Local state
  and delivered events remain responsive; permanently stuck hosts need restarting.
- Audio: native notifications; tick batches are summed behind one worker per source.
  The native command queue is bounded to 32 requests. Startup and write synchronization
  have two-second deadlines; failed connections retry after 500 ms. Absolute percentage
  writes implement the above-cap clamp.
- Workspace: socket2 events plus 2-second reconciliation, rediscovery after socket
  replacement; one running movement and latest pending direction only.
- Theme: replacement-safe path polling every 250–500 ms with 400 ms settling;
  healthy palette reconciliation is due after five seconds, plus scheduling/query
  time. No hook installation required.
- Night/idle Record: 2 seconds; active Record: 1 second. Accelerated post-command
  reads at completion and 0.3, 1.0 and 2.5 seconds later.
- Failed state polling backs off to 10 seconds; event connections to 30 seconds;
  host reconnection to 10 seconds. Failed audio batches are consumed, not retried;
  subsequently accepted inputs form separate batches.
- Failure indication uses rendered `ERROR` for two seconds; no dependence on
  unverified host `showAlert` support.

## Verification status

Display backpressure verification (0.2.4): six deterministic scheduler tests cover
latest replacement, fairness, same-context generations, reverting to the last
image during a write, command capacity/expiration/uncertain delivery, transport
failure, and session cancellation without command replay. The real-binary harness
fills a non-reading host's TCP buffers until `display stalled` is observed,
verifies lifecycle removal and a local button command still complete, resumes
reading and checks current audio state, then closes a second stalled connection
and verifies reconnection/redraw. Strict Clippy and Rust/Python suites pass.
Installed release matches the built executable. Live profile switching passed in
both directions after host startup; an immediate post-restart attempt ran before
registration completed and failed. Socket queues were empty after verification.
These checks establish plugin resilience; the original OpenDeck freeze's root
cause and physical-button acceptance under that freeze remain unconfirmed.

Passed: Rust typecheck, strict Clippy, rendered-output tests (dimensions, opacity,
strict grayscale, stale/failure distinctions, recording duration formatting),
real-binary WebSocket host harness (registration, duplicates, keyDown-only launch,
audio above-cap adjustment, shared Night gates, Workspace coalescing, Theme busy
drop, panel tap, private audio-daemon recovery/no replay, host reconnect restoration).
Native audio integration tests use a private PipeWire daemon and independent CLI
observers to verify output/input volume and mute, repeated adjustments, above-cap
observation, external changes, default-device changes, unavailable defaults, and
daemon restart without replay. Tests never change the desktop audio session.
Migration/archive tests exercise isolated files, including deletion and permission
restoration. `cargo test` and `python -m unittest discover -s tests -p 'test_*.py'`
re-run the checks.

Stats checks cover CPU accounting, memory availability, disk-sector conversion,
elapsed-time throughput, reset/new-device handling, history bounds, physical
device filtering, missing-source recovery, grayscale/themed graphs, and live CPU
history updates through the real-binary host harness. Profile tests confirm the
original keys/panels and Default file are preserved, custom images are copied,
and existing System Stats profiles are not overwritten. Power tests cover
available-profile order, signed stepping, wrapping, invalid state, displayed
profile names, and unavailable indication. The real host harness uses isolated
fixtures for profile changes and speed-test launch, so checks do not change the
desktop power mode or generate speed-test traffic.

Basic appearance, live installation/registration, and Default migration have been
confirmed. Not yet fully verified: all twelve live controls, dark/light/monochrome
hardware acceptance, real backend restarts, idle CPU, observed render latency,
and complete cleanup/rollback on the live desktop.
Renderer startup validates the embedded font's required action and mute glyphs.
A killed/crashed plugin requires restarting OpenDeck.

## Confirmed limitations from source review

1. **Theme settling retry is missing.** If the theme name changes but the color
   command still returns the previous valid palette, the pair is accepted. With
   unchanged name/metadata afterward, another read waits for the approximately
   five-second reconciliation. The intended follow-up read about one second
   later is not implemented (`theme_due`, `refresh_theme`, `state::theme`).
2. **Host retry delay does not reset.** `main` sleeps 1, 2, 4, 8, then 10 seconds
   after successive disconnects, even if connections were healthy between them.
   This delays restoration; source inspection does not prove the hardware blanks.
3. **Workspace event queries are not burst-coalesced.** Every Hyprland socket line triggers
    three workspace queries. Audio now uses native notifications instead of CLI queries.
   Refresh batches are awaited serially on each watcher, while the 100 ms cadence
   limits rendering/sends rather than backend queries.

The review rejected two alleged failures: the panel `THEME?` badge occupies
x=12–61 while footer text starts at x=72, and Tokio preserves an unconsumed
`notify_one` permit when the temporary notification waiter is dropped.
These limitations are documented, not fixed by the application refactor.
