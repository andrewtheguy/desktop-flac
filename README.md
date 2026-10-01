# desktop-flac

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
- `load_from`: the folder an application carries its own libFLAC in, for one
  that does.

libFLAC is not linked: the shared library is loaded at run time, FLAC 1.5's or
1.4's — `libFLAC.so.14` or `.so.12` on Linux, `libFLAC.14.dylib` or `.12.dylib`
on macOS, `libFLAC.dll` on Windows — so nothing of FLAC is needed to build.

Use it by release tag:

```toml
desktop-flac = { git = "https://github.com/andrewtheguy/desktop-flac", tag = "v0.0.1" }
```
