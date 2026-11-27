# Third-party credits

Ideas, parameters or code taken from other projects, with their licenses. Add an entry whenever something is ported.

| Project | Author | License | What we use | Where |
| --- | --- | --- | --- | --- |
| [Sable](https://github.com/ryanhcode/sable) | RyanHCode | PolyForm Shield 1.0.0 (not open source; **ideas only, no code**, source not read) | Reference for moving structures (ADR-0019): structures as small worlds with a pose ("sub-levels"), Rapier physics, per-block physics data, per-dimension physics, entities riding relative to a structure | design only |
| [Create: Aeronautics (Simulated Project)](https://github.com/Creators-of-Aeronautics/Simulated-Project) | Creators of Aeronautics | not checked (ideas only until checked) | Reference for the gameplay of the industrial build: assembly, propellers, balloons, floating blocks | design only |
| [Tectonic](https://modrinth.com/datapack/tectonic) | Apollo (`Apollounknowndev`) | MIT (declared on Modrinth; the source repository has no LICENSE file yet — copyright line to be confirmed before any verbatim copy) | Reference for the extended world generator (T1-17): terrain-shaping ideas and parameters | not ported yet |

## Fonts shipped in the client binary

egui's default fonts (crate `epaint_default_fonts`, enabled by the `default_fonts` feature for the menu, T1-13). Their license texts ship next to the binary in a release.

| Font | License |
| --- | --- |
| Hack Regular (Source Foundry Authors; derived from Bitstream Vera / DejaVu) | MIT + Bitstream Vera License |
| Ubuntu Light (Canonical) | Ubuntu Font Licence 1.0 |
| Noto Emoji Regular (Google) | SIL Open Font License 1.1 |
| emoji-icon-font (John Slegers) | MIT |
| egui-icons (a subset of emoji-icon-font) | MIT |
