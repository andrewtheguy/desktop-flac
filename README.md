# sound-flac

FLAC for a desktop's sound, as [wlshare](https://github.com/andrewtheguy/wlshare)
and the [remotex](https://github.com/andrewtheguy/remotex) gateway both code it.

- `Stream`: what the two ends agree on before any sound — the rate, the
  channels, the sample width and the block, the frames of samples in every FLAC
  frame.
- `Encoder`: one FLAC frame of every block, made the moment the block is whole.
  libFLAC holds a block back until it has a sample of the next, so each frame
  is a stream of its own, one block long, and is numbered zero.
- `Decoder`: one frame in, its block out, each frame on its own, behind the
  stream header it builds from the `Stream`; a frame of any other shape, or
  that is not one whole frame, is an error.

libFLAC is linked statically, FLAC 1.5 from the prebuilt archive
[libflac-prebuilt](https://github.com/andrewtheguy/libflac-prebuilt) publishes
for Linux x86_64 and aarch64, macOS arm64 and Windows x86_64: the build
downloads it and compiles no C, and nothing of FLAC is installed or loaded
where the binary runs.

Use it by release tag:

```toml
sound-flac = { git = "https://github.com/andrewtheguy/sound-flac", tag = "v0.0.4" }
```
