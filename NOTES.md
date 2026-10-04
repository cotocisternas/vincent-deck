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
| `src/state.rs` | Desktop queries, parsing, and snapshot accessors |
| `src/process.rs` | Command environment, sessions, query timeout/output limits |
| `src/render.rs` | Action styles, palettes, elapsed text, font rasterization, PNG drawing |
| `scripts/manage.py` | Build/install, manifest generation, profile migration, rollback |

The application refactor separates polling and theme checks, refresh operations,
render passes, audio batches, busy gates, command execution, and event watchers.
`Surface` owns visible per-context flags and send deduplication; `RenderCache`
owns the bounded image cache. Snapshot accessors centralize field selection.
Content is still a common struct containing fields for all actions; action policy
dispatch remains in several matches.

## Runtime limits

- State-query timeout: 2 seconds; captured query stdout capped at 256 KiB.
  Interactive launchers/selectors have no timeout. Commands run in separate
  sessions and children are waited asynchronously.
- Cache: at most 64 rendered data URIs; per-context last-sent image is bounded by
  visible instances. Window/theme input strings are bounded. Recording cache inputs
  contain formatted visible elapsed time only.
- Panel send limit: 100 ms minimum per context (at most 10 Hz); fixed-cadence
  refresh prevents sustained event streams starving display updates.
- Audio: pactl events plus 500 ms reconciliation; tick batches are summed behind
  one worker per source. Absolute percentage writes implement the above-cap clamp.
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

Passed: Rust typecheck, strict Clippy, rendered-output tests (dimensions, opacity,
strict grayscale, stale/failure distinctions, recording duration formatting),
real-binary WebSocket host harness (registration, duplicates, keyDown-only launch,
audio above-cap adjustment, shared Night gates, Workspace coalescing, Theme busy
drop, panel tap, missing-tool recovery/no replay, reconnect restoration).
Migration/archive tests exercise isolated files, including deletion and permission
restoration. `cargo test` and `python -m unittest discover -s tests -p 'test_*.py'`
re-run the checks.

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
3. **Event queries are not burst-coalesced.** Every Hyprland socket line triggers
   three workspace queries; every matching pactl line triggers two audio queries.
   Refresh batches are awaited serially on each watcher, while the 100 ms cadence
   limits rendering/sends rather than backend queries.

The review rejected two alleged failures: the panel `THEME?` badge occupies
x=12–61 while footer text starts at x=72, and Tokio preserves an unconsumed
`notify_one` permit when the temporary notification waiter is dropped.
These limitations are documented, not fixed by the application refactor.
