# Use Rust and OpenAction for Vincent Deck

Status: accepted

Vincent Deck will use Rust and OpenAction, following the native plugin integration
already documented for the target OpenDeck installation. This trades freedom to
choose another language or protocol stack for a consistent implementation path
with local precedent and no additional language runtime. The user confirmed this
decision during the design interview on 2026-10-03.

Rendering libraries remain provisional until one key and one panel demonstrate
perceptual fidelity. Choosing Rust and OpenAction does not require pixel-identical
output to the legacy ImageMagick renderer or freeze the proposed internal structure.
