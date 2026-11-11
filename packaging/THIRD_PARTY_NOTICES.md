# Third-party software

Melos is licensed under GPL-3.0-or-later. Dependency versions are frozen
in Cargo.lock; their original licenses and copyright notices remain applicable.

Principal dependencies include:

| Dependency | License |
| --- | --- |
| GPUI and gpui-unofficial | Apache-2.0 |
| Symphonia | MPL-2.0 |
| CPAL | Apache-2.0 |
| Reqwest, Tokio, Serde, windows-rs | MIT OR Apache-2.0 |

The GPUI text input implementation derives from Zed's GPUI input example:
https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/input.rs
Copyright Zed Industries. Licensed under Apache-2.0.

The repository's generated test audio is original and dedicated to the
public domain under CC0-1.0; it is not bundled with the player.

Complete transitive-license review remains a prerequisite for a public release.
