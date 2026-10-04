# OpenDeck profile-switch permission

Stock OpenDeck 2.14.0 silently rejects `switchProfile` messages unless they come
from Starter Pack or the alternative Elgato device implementation. Sending the
same JSON from Vincent Deck is not sufficient. This caused dial-clicks to appear
in the UI without changing profiles; screen taps continued working.

The owner approved a narrow host patch adding `dev.vincent.deck.sdPlugin` to the
profile-switch allowlist. Device-brightness permissions and other plugins remain
unchanged. The patch is preserved in
`patches/opendeck-2.14.0-vincent-profile-switch.patch`.

## Rebuild

Use an OpenDeck checkout at tag `v2.14.0` (commit
`b2d09ca60089cea38ffea7eef191270ffefdf851`), Deno, Cargo, and Tauri's Linux build
dependencies (GTK3, WebKitGTK 4.1, libsoup3, libudev, OpenSSL, appindicator).
From that checkout:

```sh
git apply /path/to/vincent-deck/patches/opendeck-2.14.0-vincent-profile-switch.patch
deno install
deno task build
```

Then run these commands from the checkout's `src-tauri/` directory (its build
script uses relative paths):

```sh
cargo test profile_switch_tests --features custom-protocol
cargo build --release --features custom-protocol
```

Copy `src-tauri/target/release/opendeck` and the generated
`src-tauri/target/plugins/` directory into the same installation directory.

## Local installation

The patched host is installed at
`~/.local/share/opendeck/vincent-build/opendeck`, with bundled Starter Pack assets
beside it under `plugins/`. `~/.local/bin/opendeck` launches that executable.
The per-user desktop entry and `~/.config/autostart/opendeck.desktop` also use it.
The packaged `/usr/bin/opendeck` remains available and unmodified.

This patch is required for Vincent Deck's combined Theme/Network panels to switch
profiles. A normal plugin rebuild does not patch a stock OpenDeck installation.
Future host upgrades need the permission change ported and the host rebuilt.

## Live regression check

With OpenDeck, the plugin, and the Stream Deck + running, select `default` or
`performance`, then run from Vincent Deck's repository:

```sh
cargo test --test live_profile_switch -- --ignored --nocapture
```

The test injects rightmost encoder down/up events into the real host and asserts
that its persisted selected profile changes. Run twice to verify both directions
and return to the starting page. `OPENDECK_PORT` overrides the default port 57116.
It is ignored during normal tests because it changes the live profile.

The same test failed against the stock host and passed in both directions against
the patched host. The ordinary mock WebSocket harness only verifies the message
sent by the plugin; it cannot establish host authorization or UI switching.

## Roll back the host

Close the running host, start `/usr/bin/opendeck`, remove the user launcher
`~/.local/bin/opendeck` and desktop override
`~/.local/share/applications/opendeck.desktop`, and restore the original autostart
entry from
`~/.local/state/vincent-deck/opendeck-autostart-before-host-patch.desktop`.
The built host may also have updated `opendeck-handler.desktop`; change its Exec
back to `/usr/bin/opendeck` if retaining the packaged host for URL handling.
Profiles and Vincent Deck's other controls are unaffected by host rollback,
but combined dial-click profile switching will again be blocked.
