# Bundled renderer font

`TerminessNerdFontMono-Bold.ttf` is the unchanged font previously used by the
renderer from the system installation. It includes the action and mute glyphs.

- Upstream: [Nerd Fonts, version 3.5.1](https://github.com/ryanoasis/nerd-fonts/tree/v3.5.1/patched-fonts/Terminus).
- Copied from Arch Linux package `ttf-terminus-nerd` version `3.5.1-2`.
- Size: 2,749,228 bytes (approximately 2.62 MiB).
- SHA-256: `cf1fb6943644d01fb8032a56e542af4c207d60b4d0eb80eb89cc0690ca3ae01a`.
- Font license: SIL Open Font License 1.1, with the copyright and reserved-name
  notices in `LICENSE.txt`. `NERD-FONTS-LICENSE.txt` also preserves the Nerd Fonts
  project's licensing and attribution notice. The font is not covered by the
  application's MIT license.

`src/render.rs` embeds the TTF with `include_bytes!`; the executable and renderer
tests never load a font from the filesystem. Keep the font and notices together
when redistributing. `scripts/manage.py install` copies this directory into the
generated and installed plugin bundles along with the other assets.
