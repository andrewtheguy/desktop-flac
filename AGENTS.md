# Repository instructions

- Strict no backward-compatibility or legacy paths no matter what.
- One crate, `desktop-flac`: the one place libFLAC is spoken to for the wlshare
  daemon and the remotex gateway, which each pin it by a release tag of this
  repository. How a block becomes a frame, how a frame is read back, which
  libFLAC is loaded and from where change here and reach them as a pin bump.
  Nothing about a wire belongs here: how a frame is framed in a message, how a
  stream's shape is agreed and what a sample's bytes are belong to each user.
- libFLAC is loaded at run time and never linked: FLAC 1.5 and 1.4, the two the
  systems the users are built for have. Building needs nothing of FLAC; the
  tests need the system's libFLAC installed (`libflac14` or `libflac12t64` on
  Debian and Ubuntu, `brew install flac` on macOS).
- After changes run `cargo test` and `cargo clippy --all-targets -- -D warnings`.
  The encoder gets an independent decoder in its tests, and the decoder an
  independent encoder.
- `./scripts/test-libflac.sh` runs both against FLAC 1.5 and 1.4 in turn, each
  in a Debian container (podman) that has it: run it after a change to what is
  called in libFLAC.
- Do not run `cargo fmt`. Errors are `thiserror`: every caller branches on them.
- A release is the version in `Cargo.toml` bumped and tagged `v<version>` on
  `main`; users pin the tag.
