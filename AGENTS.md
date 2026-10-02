# Repository instructions

- Strict no backward-compatibility or legacy paths no matter what.
- One crate, `desktop-flac`: the one place libFLAC is spoken to for the wlshare
  daemon and the remotex gateway, which each pin it by a release tag of this
  repository. How a block becomes a frame, how a frame is read back and which
  libFLAC is linked change here and reach them as a pin bump.
  Nothing about a wire belongs here: how a frame is framed in a message, how a
  stream's shape is agreed and what a sample's bytes are belong to each user.
- libFLAC is linked statically, from the prebuilt archive
  `libflac-prebuilt-sys` downloads: building compiles no C and needs no libFLAC
  installed, and nothing is loaded at run time. Another FLAC release is that
  crate's tag bumped in `Cargo.toml`.
- After changes run `cargo test` and `cargo clippy --all-targets -- -D warnings`.
  The encoder gets an independent decoder in its tests, and the decoder an
  independent encoder.
- Do not run `cargo fmt`. Errors are `thiserror`: every caller branches on them.
- A release is the version in `Cargo.toml` bumped and tagged `v<version>` on
  `main`; users pin the tag.
