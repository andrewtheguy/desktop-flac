//! FLAC for a desktop's sound, as wlshare and the remotex gateway both code it.
//!
//! wlshare sends what its desktop plays as FLAC and the gateway decodes it, so
//! the sound is lossless between them. Both ends are libFLAC, the reference
//! codec, and this crate is the one place it is spoken to:
//!
//! - **[`Stream`]** is what the two ends agree on before any sound: the rate,
//!   the channels, the sample width and the block, the frames of samples in
//!   every FLAC frame.
//! - **[`Encoder`]** makes one FLAC frame of every block, the moment the block
//!   is whole.
//! - **[`Decoder`]** takes one frame and gives the block back, each frame on its
//!   own: a frame lost on the way costs its own samples and nothing after.
//!
//! libFLAC is not linked. The system's shared library is loaded the first time
//! a stream is set up, or the one an application carries when it says where
//! ([`load_from`]), so the crate builds where there is no FLAC and a user ships
//! none of it it does not choose to.
//!
//! ## A frame is a stream of its own
//!
//! libFLAC encodes a stream, and holds every block back until it has a sample
//! of the next one, to know whether the block is the stream's last. Sound heard
//! as it is made cannot wait a block for that, so [`Encoder`] makes each frame
//! a stream of its own, one block long: finishing a stream encodes what it
//! holds. The marker and metadata such a stream opens with are dropped, and its
//! only frame is numbered zero.
//!
//! The stream header, `STREAMINFO`, is never sent either: everything in it is
//! in the [`Stream`] both ends hold, so [`Decoder`] builds it
//! ([`Stream::streaminfo`]) and reads each frame behind it as a stream of its
//! own again. A frame states its own rate unless no frame header has a code for
//! it, and then it leaves the rate to that header.
//!
//! Nothing about a wire is here: how a frame is framed in a message, how the
//! stream's shape is agreed and what a sample's bytes are on either side belong
//! to the user.

mod libflac;

use std::ffi::{c_int, c_void};
use std::path::PathBuf;
use std::ptr;

pub use libflac::{load, load_from};
use thiserror::Error;

/// The bytes of a `STREAMINFO` block.
pub const STREAMINFO_LEN: usize = 34;

/// What both ends agree on before any sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    /// Samples per second per channel, at most the 20 bits `STREAMINFO` has.
    pub rate: u32,
    /// 1 to 8.
    pub channels: u8,
    /// The bits in a sample, 4 to 24.
    pub bits: u8,
    /// The frames of samples in every FLAC frame, 16 or more.
    pub block: u16,
}

impl Stream {
    /// Whether this is a stream FLAC carries.
    pub fn check(&self) -> Result<(), Error> {
        let carried = (1..1 << 20).contains(&self.rate)
            && (1..=8).contains(&self.channels)
            && (4..=24).contains(&self.bits)
            && self.block >= 16;
        if carried { Ok(()) } else { Err(Error::Unsupported(*self)) }
    }

    /// The samples in a block: a frame of them for each channel.
    pub fn samples(&self) -> usize {
        usize::from(self.block) * usize::from(self.channels)
    }

    /// The stream header a decoder reads frames behind, as the FLAC
    /// specification lays out `STREAMINFO`: the block as both the smallest and
    /// the largest, and the frame sizes, the total and the MD5 unknown.
    pub fn streaminfo(&self) -> Result<[u8; STREAMINFO_LEN], Error> {
        self.check()?;
        let block = self.block.to_be_bytes();
        let mut info = [0u8; STREAMINFO_LEN];
        info[0..2].copy_from_slice(&block);
        info[2..4].copy_from_slice(&block);
        // Rate (20 bits), channels - 1 (3), bits per sample - 1 (5), total
        // samples (36, unknown).
        let packed =
            (u64::from(self.rate) << 44) | (u64::from(self.channels - 1) << 41) | (u64::from(self.bits - 1) << 36);
        info[10..18].copy_from_slice(&packed.to_be_bytes());
        Ok(info)
    }
}

/// Why a frame could not be made or read.
#[derive(Debug, Error)]
pub enum Error {
    /// libFLAC is not where it was looked for: how to get it, and why each file
    /// tried was refused.
    #[error("libFLAC is not installed: {install} ({tried})")]
    Missing { install: String, tried: String },
    #[error("libFLAC was already loaded when {0} was named as the folder to load it from")]
    AlreadyLoaded(PathBuf),
    #[error("{0:?} is not a stream FLAC carries")]
    Unsupported(Stream),
    /// libFLAC would not start the stream, carried as its name for why.
    #[error("libFLAC refused the stream: {0}")]
    Refused(String),
    #[error("a block of {got} samples, where the stream's is {want}")]
    Block { got: usize, want: usize },
    /// libFLAC failed on a block, carried as its name for the state that left
    /// it in.
    #[error("encoding a FLAC frame: {0}")]
    Encode(String),
    /// What was handed over is not one FLAC frame, carried as libFLAC's name
    /// for what it found.
    #[error("decoding a FLAC frame: {0}")]
    Decode(String),
    #[error(
        "a FLAC frame of {frames} frames of {channels} channels of {bits} bits at {rate} Hz, where {want:?} was agreed"
    )]
    Shape { frames: u32, channels: u32, bits: u32, rate: u32, want: Stream },
}

/// One libFLAC encoder or decoder, deleted when it is dropped.
struct Handle {
    pointer: *mut c_void,
    delete: unsafe extern "C" fn(*mut c_void),
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the pointer is this value's alone, and is not used again.
        unsafe { (self.delete)(self.pointer) };
    }
}

/// One stream's encoder: a block in, a FLAC frame out.
pub struct Encoder {
    stream: Stream,
}

impl Encoder {
    /// An encoder for `stream`, if FLAC carries it and libFLAC is here to
    /// encode it: a stream is started and finished with nothing in it.
    pub fn new(stream: Stream) -> Result<Self, Error> {
        stream.check()?;
        let encoder = Self { stream };
        encoder.run(&[], &mut Vec::new())?;
        Ok(encoder)
    }

    /// The stream this encoder was made for.
    pub fn stream(&self) -> Stream {
        self.stream
    }

    /// Append to `out` the FLAC frame of one block: [`Stream::samples`]
    /// interleaved samples, each a signed value of the stream's width.
    pub fn encode(&mut self, block: &[i32], out: &mut Vec<u8>) -> Result<(), Error> {
        if block.len() != self.stream.samples() {
            return Err(Error::Block { got: block.len(), want: self.stream.samples() });
        }
        let before = out.len();
        self.run(block, out)?;
        if out.len() == before {
            return Err(Error::Encode("libFLAC finished the stream without a frame".into()));
        }
        Ok(())
    }

    /// One libFLAC stream from start to finish, holding `block`: a whole one,
    /// or nothing.
    fn run(&self, block: &[i32], out: &mut Vec<u8>) -> Result<(), Error> {
        let api = libflac::api()?;
        let stream = &self.stream;
        // SAFETY: the calls are made as libFLAC's headers document them, on an
        // encoder that is live until it is dropped. `block` is empty or a whole
        // block, and `out` outlives the stream, whose writes are all inside
        // these calls.
        unsafe {
            let encoder = Handle { pointer: (api.encoder_new)(), delete: api.encoder_delete };
            if encoder.pointer.is_null() {
                return Err(Error::Encode("libFLAC could not allocate an encoder".into()));
            }
            // A setter refuses only an encoder already started, which a new one
            // is not; what it is given is judged when the stream starts. The
            // streamable subset has every frame state its rate, and a rate no
            // frame header can state is left to the stream header the decoder
            // builds, so it is off. So is the MD5 of the samples, which only a
            // stream header that is sent would carry.
            (api.encoder_set_streamable_subset)(encoder.pointer, 0);
            (api.encoder_set_do_md5)(encoder.pointer, 0);
            (api.encoder_set_channels)(encoder.pointer, u32::from(stream.channels));
            (api.encoder_set_bits_per_sample)(encoder.pointer, u32::from(stream.bits));
            (api.encoder_set_sample_rate)(encoder.pointer, stream.rate);
            (api.encoder_set_blocksize)(encoder.pointer, u32::from(stream.block));
            let status = (api.encoder_init_stream)(
                encoder.pointer,
                keep_frame,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::from_mut(out).cast(),
            );
            if status != 0 {
                return Err(Error::Refused(api.encoder_init_status(status)));
            }
            let state = || libflac::state((api.encoder_get_resolved_state_string)(encoder.pointer));
            if !block.is_empty()
                && (api.encoder_process_interleaved)(encoder.pointer, block.as_ptr(), u32::from(stream.block)) == 0
            {
                return Err(Error::Encode(state()));
            }
            // A failed finish leaves the encoder in the state that failed it.
            if (api.encoder_finish)(encoder.pointer) == 0 {
                return Err(Error::Encode(state()));
            }
        }
        Ok(())
    }
}

/// The encoder's stream as libFLAC writes it: a frame is kept, in the
/// `Vec<u8>` that `client` is, and the marker and metadata a stream opens with
/// are not.
unsafe extern "C" fn keep_frame(
    _encoder: *const c_void,
    buffer: *const u8,
    bytes: usize,
    samples: u32,
    _frame: u32,
    client: *mut c_void,
) -> c_int {
    if samples > 0 {
        // SAFETY: `client` is the vector `Encoder::run` lent for the stream's
        // length, which nothing else touches until it has finished, and
        // `buffer` holds `bytes` bytes for the length of this call.
        unsafe { (*client.cast::<Vec<u8>>()).extend_from_slice(std::slice::from_raw_parts(buffer, bytes)) };
    }
    libflac::WRITE_CONTINUE
}

/// The stream marker and the header of a `STREAMINFO` block that is the last
/// of the metadata, which the block itself follows.
const STREAM_OPENING: [u8; 8] = [b'f', b'L', b'a', b'C', 0x80, 0, 0, STREAMINFO_LEN as u8];

/// One stream's decoder: a FLAC frame in, its block out.
pub struct Decoder {
    stream: Stream,
    /// What every frame is read behind: [`STREAM_OPENING`] and the stream's
    /// `STREAMINFO`.
    opening: [u8; STREAM_OPENING.len() + STREAMINFO_LEN],
}

impl Decoder {
    /// A decoder for `stream`, if FLAC carries it and libFLAC is here to decode
    /// it.
    pub fn new(stream: Stream) -> Result<Self, Error> {
        let mut opening = [0; STREAM_OPENING.len() + STREAMINFO_LEN];
        opening[..STREAM_OPENING.len()].copy_from_slice(&STREAM_OPENING);
        opening[STREAM_OPENING.len()..].copy_from_slice(&stream.streaminfo()?);
        libflac::api()?;
        Ok(Self { stream, opening })
    }

    /// The stream this decoder was made for.
    pub fn stream(&self) -> Stream {
        self.stream
    }

    /// Append to `out` the block one FLAC frame holds: [`Stream::samples`]
    /// interleaved samples, each a signed value of the stream's width, bit for
    /// bit what was encoded. `frame` is that frame and nothing else, of the
    /// stream's own shape; libFLAC checks both of its CRCs.
    pub fn decode(&mut self, frame: &[u8], out: &mut Vec<i32>) -> Result<(), Error> {
        let api = libflac::api()?;
        let before = out.len();
        let mut reading = Reading { api, want: self.stream, rest: [&self.opening, frame], out, frames: 0, fault: None };
        // SAFETY: the calls are made as libFLAC's headers document them, on a
        // decoder that is live until it is dropped. `reading` outlives the
        // stream, whose callbacks are all inside these calls and are the only
        // thing to touch it until then.
        let state = unsafe {
            let decoder = Handle { pointer: (api.decoder_new)(), delete: api.decoder_delete };
            if decoder.pointer.is_null() {
                return Err(Error::Decode("libFLAC could not allocate a decoder".into()));
            }
            let status = (api.decoder_init_stream)(
                decoder.pointer,
                read_stream,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                take_frame,
                ptr::null(),
                note_error,
                ptr::from_mut(&mut reading).cast(),
            );
            if status != 0 {
                return Err(Error::Refused(api.decoder_init_status(status)));
            }
            // What it returns is in the state, and in what the callbacks noted.
            (api.decoder_process_until_end_of_stream)(decoder.pointer);
            let state = libflac::state((api.decoder_get_resolved_state_string)(decoder.pointer));
            (api.decoder_finish)(decoder.pointer);
            state
        };
        let Reading { out, frames, fault, .. } = reading;
        let fault = match (fault, frames) {
            (Some(fault), _) => fault,
            (None, 1) => return Ok(()),
            // A frame cut short is the stream's end to libFLAC, not an error.
            (None, _) => Error::Decode(format!("no whole frame, and the decoder at {state}")),
        };
        out.truncate(before);
        Err(fault)
    }
}

/// One frame being read: what the decoder's callbacks share.
struct Reading<'a> {
    api: &'static libflac::Api,
    want: Stream,
    /// The stream not yet read: the opening, then the frame.
    rest: [&'a [u8]; 2],
    out: &'a mut Vec<i32>,
    /// The frames decoded so far.
    frames: u32,
    /// The first thing that went wrong.
    fault: Option<Error>,
}

/// The decoder's stream as libFLAC reads it: the opening, the frame, and then
/// its end.
unsafe extern "C" fn read_stream(
    _decoder: *const c_void,
    buffer: *mut u8,
    bytes: *mut usize,
    client: *mut c_void,
) -> c_int {
    // SAFETY: `client` is the `Reading` `Decoder::decode` lent for the stream's
    // length, and `buffer` has room for the `*bytes` libFLAC asks for.
    unsafe {
        let reading = &mut *client.cast::<Reading>();
        let Some(part) = reading.rest.iter_mut().find(|part| !part.is_empty()) else {
            *bytes = 0;
            return libflac::READ_END;
        };
        let taken = part.len().min(*bytes);
        ptr::copy_nonoverlapping(part.as_ptr(), buffer, taken);
        *part = &part[taken..];
        *bytes = taken;
    }
    libflac::READ_CONTINUE
}

/// A decoded frame: kept if it is the stream's first and of its shape.
unsafe extern "C" fn take_frame(
    _decoder: *const c_void,
    frame: *const libflac::FrameHeader,
    buffer: *const *const i32,
    client: *mut c_void,
) -> c_int {
    // SAFETY: `client` is the `Reading` `Decoder::decode` lent for the stream's
    // length; `frame` starts with the header read here, and `buffer` holds a
    // pointer for each of its channels to `blocksize` samples, all for the
    // length of this call.
    unsafe {
        let reading = &mut *client.cast::<Reading>();
        let header = &*frame;
        let want = reading.want;
        let agreed = header.blocksize == u32::from(want.block)
            && header.channels == u32::from(want.channels)
            && header.bits_per_sample == u32::from(want.bits)
            && header.sample_rate == want.rate;
        if !agreed {
            reading.fault.get_or_insert(Error::Shape {
                frames: header.blocksize,
                channels: header.channels,
                bits: header.bits_per_sample,
                rate: header.sample_rate,
                want,
            });
            return libflac::WRITE_ABORT;
        }
        reading.frames += 1;
        if reading.frames > 1 {
            reading.fault.get_or_insert(Error::Decode("more than one frame".into()));
            return libflac::WRITE_ABORT;
        }
        let channels = std::slice::from_raw_parts(buffer, usize::from(want.channels));
        reading.out.reserve(want.samples());
        for at in 0..usize::from(want.block) {
            reading.out.extend(channels.iter().map(|channel| *channel.add(at)));
        }
    }
    libflac::WRITE_CONTINUE
}

/// What libFLAC found that is not FLAC: the first is what the frame is refused
/// for.
unsafe extern "C" fn note_error(_decoder: *const c_void, status: c_int, client: *mut c_void) {
    // SAFETY: `client` is the `Reading` `Decoder::decode` lent for the stream's
    // length.
    let reading = unsafe { &mut *client.cast::<Reading>() };
    reading.fault.get_or_insert_with(|| Error::Decode(reading.api.decoder_error_status(status)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO: Stream = Stream { rate: 48_000, channels: 2, bits: 16, block: 960 };

    /// A tone with noise on it, interleaved: every sample of it uses the full
    /// range of the stream's width.
    fn signal(stream: Stream, blocks: usize) -> Vec<i32> {
        let mut seed = 0x2545_f491_u32;
        let mut out = Vec::with_capacity(blocks * stream.samples());
        for n in 0..blocks * usize::from(stream.block) {
            for c in 0..stream.channels {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let t = n as f64 / f64::from(stream.rate);
                let tone = (t * 440.0 * (1.0 + f64::from(c)) * std::f64::consts::TAU).sin() * 0.7;
                let noise = (f64::from(seed) / f64::from(u32::MAX) - 0.5) * 0.2;
                let full = ((tone + noise).clamp(-1.0, 1.0) * f64::from(i32::MAX)) as i32;
                out.push(full >> (32 - u32::from(stream.bits)));
            }
        }
        out
    }

    /// A frame of every block, as [`Encoder`] makes them.
    fn encode(stream: Stream, samples: &[i32]) -> Vec<Vec<u8>> {
        let mut encoder = Encoder::new(stream).unwrap();
        samples
            .chunks(stream.samples())
            .map(|block| {
                let mut frame = Vec::new();
                encoder.encode(block, &mut frame).unwrap();
                frame
            })
            .collect()
    }

    /// The frames' samples by symphonia's decoder, which shares nothing with
    /// libFLAC, behind the header [`Stream::streaminfo`] builds.
    fn symphonia(stream: Stream, frames: &[Vec<u8>]) -> Vec<i32> {
        use symphonia_core::codecs::audio::well_known::CODEC_ID_FLAC;
        use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder as _, AudioDecoderOptions};
        use symphonia_core::packet::Packet;
        use symphonia_core::units::{Duration, Timestamp};

        let mut params = AudioCodecParameters::new();
        params.for_codec(CODEC_ID_FLAC).with_extra_data(Box::new(stream.streaminfo().unwrap()));
        let mut decoder =
            symphonia_bundle_flac::FlacDecoder::try_new(&params, &AudioDecoderOptions::default()).unwrap();
        let mut out = Vec::new();
        let mut samples = Vec::<i32>::new();
        for frame in frames {
            let packet =
                Packet::new(0, Timestamp::new(0), Duration::new(u64::from(stream.block)), frame.clone());
            let decoded = decoder.decode(&packet).unwrap();
            assert_eq!(decoded.frames(), usize::from(stream.block));
            assert_eq!(decoded.spec().channels().count(), usize::from(stream.channels));
            decoded.copy_to_vec_interleaved(&mut samples);
            // symphonia scales to 32 bits; back down to the stream's width.
            out.extend(samples.iter().map(|sample| sample >> (32 - u32::from(stream.bits))));
        }
        out
    }

    /// A frame of every block by flacenc, which shares nothing with libFLAC,
    /// numbered from `first`.
    fn flacenc(stream: Stream, samples: &[i32], first: usize) -> Vec<Vec<u8>> {
        use flacenc::bitsink::ByteSink;
        use flacenc::component::{BitRepr as _, StreamInfo};
        use flacenc::error::Verify as _;
        use flacenc::source::{Fill as _, FrameBuf};

        let block = usize::from(stream.block);
        let mut info =
            StreamInfo::new(stream.rate as usize, usize::from(stream.channels), usize::from(stream.bits)).unwrap();
        info.set_block_sizes(block, block).unwrap();
        let config = flacenc::config::Encoder::default().into_verified().unwrap();
        let mut framebuf = FrameBuf::with_size(usize::from(stream.channels), block).unwrap();
        samples
            .chunks(stream.samples())
            .enumerate()
            .map(|(n, chunk)| {
                framebuf.fill_interleaved(chunk).unwrap();
                let frame = flacenc::encode_fixed_size_frame(&config, &framebuf, first + n, &info).unwrap();
                let mut sink = ByteSink::new();
                frame.write(&mut sink).unwrap();
                sink.into_inner()
            })
            .collect()
    }

    /// The frames' samples by [`Decoder`].
    fn decode(stream: Stream, frames: &[Vec<u8>]) -> Vec<i32> {
        let mut decoder = Decoder::new(stream).unwrap();
        let mut out = Vec::new();
        for frame in frames {
            decoder.decode(frame, &mut out).unwrap();
        }
        out
    }

    /// The header's fields where the FLAC specification puts them, read back
    /// bit by bit rather than with the packing that wrote them.
    #[test]
    fn the_streaminfo_has_the_specified_layout() {
        let info = STEREO.streaminfo().unwrap();
        assert_eq!(&info[0..4], &[0x03, 0xC0, 0x03, 0xC0], "960 at both ends");
        assert_eq!(&info[4..10], &[0; 6], "frame sizes unknown");
        // 48000 = 0x0BB80 in 20 bits, then 001 for two channels, then 01111 for
        // 16 bits, then a total of zero.
        assert_eq!(&info[10..14], &[0x0B, 0xB8, 0x02, 0xF0]);
        assert_eq!(&info[14..34], &[0; 20]);
    }

    /// A stream FLAC has no field for has no header, encoder or decoder: no
    /// channels would underflow the header's field, and a rate past 20 bits
    /// would spill into the ones after it.
    #[test]
    fn a_stream_flac_does_not_carry_is_refused_everywhere() {
        for stream in [
            Stream { channels: 0, ..STEREO },
            Stream { channels: 9, ..STEREO },
            Stream { rate: 0, ..STEREO },
            Stream { rate: 1 << 20, ..STEREO },
            Stream { bits: 3, ..STEREO },
            Stream { bits: 32, ..STEREO },
            Stream { block: 15, ..STEREO },
        ] {
            assert!(matches!(stream.streaminfo(), Err(Error::Unsupported(s)) if s == stream));
            assert!(matches!(Encoder::new(stream), Err(Error::Unsupported(s)) if s == stream));
            assert!(matches!(Decoder::new(stream), Err(Error::Unsupported(s)) if s == stream));
        }
    }

    /// Every width, channel count and a spread of rates: what the encoder makes
    /// is read back bit for bit by symphonia, and by the decoder here. 70001 Hz
    /// is a rate no frame header can state, so its frames leave it to the
    /// stream header.
    #[test]
    fn the_encoder_round_trips_bit_for_bit() {
        for bits in [8, 16, 24] {
            for channels in [1, 2] {
                for rate in [8_000, 11_025, 44_100, 48_000, 70_001, 96_000] {
                    let stream = Stream { rate, channels, bits, block: (rate / 50) as u16 };
                    let samples = signal(stream, 4);
                    let frames = encode(stream, &samples);
                    assert_eq!(frames.len(), 4, "{stream:?}");
                    assert!(symphonia(stream, &frames) == samples, "{stream:?} did not survive symphonia");
                    assert!(decode(stream, &frames) == samples, "{stream:?} did not survive the decoder");
                }
            }
        }
    }

    /// What flacenc makes is read back bit for bit by the decoder, whatever
    /// number a frame carries: each is read on its own.
    #[test]
    fn the_decoder_reads_an_independent_encoder_bit_for_bit() {
        for bits in [8, 16, 24] {
            for channels in [1, 2] {
                for rate in [8_000, 44_100, 48_000, 96_000] {
                    let stream = Stream { rate, channels, bits, block: (rate / 50) as u16 };
                    let samples = signal(stream, 4);
                    let frames = flacenc(stream, &samples, 1 << 20);
                    assert!(decode(stream, &frames) == samples, "{stream:?}");
                }
            }
        }
    }

    /// A folder named as the one to load libFLAC from is the only place looked
    /// in: one that holds none is an error naming it, whatever the system has.
    #[test]
    fn a_named_folder_without_libflac_is_refused() {
        let dir = std::env::temp_dir().join(format!("desktop-flac-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = load_from(&dir).expect_err("an empty folder");
        std::fs::remove_dir(&dir).unwrap();
        assert!(matches!(error, Error::Missing { .. }), "{error}");
        assert!(error.to_string().contains(&dir.display().to_string()), "{error}");
    }

    /// A frame's header says what was agreed, each field read where the FLAC
    /// specification puts it: fixed blocking, the stream's block, rate,
    /// channels and sample width, and the number zero, every frame being a
    /// stream of its own.
    #[test]
    fn a_frame_header_states_the_stream() {
        let frames = encode(STEREO, &signal(STEREO, 2));
        assert_eq!(frames.len(), 2);
        for frame in &frames {
            assert_eq!(&frame[..2], &[0xFF, 0xF8], "the sync code, and fixed blocking");
            assert_eq!(frame[2], 0x7A, "a block size of 16 bits follows the number, at 48 kHz");
            assert!(matches!(frame[3] >> 4, 0x1 | 0x8..=0xA), "two channels, however they are paired");
            assert_eq!(frame[3] & 0x0F, 0x08, "16 bits a sample");
            assert_eq!(frame[4], 0, "numbered zero");
            assert_eq!(&frame[5..7], &959u16.to_be_bytes(), "960 frames, less one");
        }
    }

    /// Silence, the state a desktop is in most of the time, is a few bytes a
    /// frame rather than the 3840 of its samples.
    #[test]
    fn silence_is_a_few_bytes() {
        let silence = vec![0; STEREO.samples()];
        let frames = encode(STEREO, &silence);
        assert!(frames[0].len() < 24, "{} bytes", frames[0].len());
        assert_eq!(symphonia(STEREO, &frames), silence);
    }

    #[test]
    fn a_block_of_another_length_is_refused() {
        let mut encoder = Encoder::new(STEREO).unwrap();
        let mut out = Vec::new();
        let short = vec![0; STEREO.samples() - 2];
        assert!(matches!(encoder.encode(&short, &mut out), Err(Error::Block { got: 1918, want: 1920 })));
        assert!(matches!(encoder.encode(&[], &mut out), Err(Error::Block { got: 0, want: 1920 })));
        assert!(out.is_empty());
    }

    /// A frame lost on the way costs its own samples and nothing after it: the
    /// next decodes as though nothing happened.
    #[test]
    fn a_missing_frame_costs_only_its_own_samples() {
        let samples = signal(STEREO, 3);
        let mut frames = flacenc(STEREO, &samples, 0);
        frames.remove(1);
        let block = STEREO.samples();
        assert_eq!(decode(STEREO, &frames), [&samples[..block], &samples[2 * block..]].concat());
    }

    /// What is not one frame of the stream's shape is an error and no samples,
    /// and the decoder is still good for the frame after it.
    #[test]
    fn what_is_not_one_frame_of_the_stream_is_refused() {
        let samples = signal(STEREO, 2);
        let good = flacenc(STEREO, &samples, 0);
        let mut decoder = Decoder::new(STEREO).unwrap();
        let mut out = vec![7];
        let mut refused = |frame: &[u8], what: &str| -> Error {
            let error = decoder.decode(frame, &mut out).expect_err(what);
            assert_eq!(out, [7], "{what}: nothing is kept of a refused frame");
            error
        };

        assert!(matches!(refused(&[0xAB; 64], "not a frame"), Error::Decode(_)));
        assert!(matches!(refused(&[], "nothing"), Error::Decode(_)));
        assert!(matches!(refused(&good[0][..good[0].len() / 2], "half a frame"), Error::Decode(_)));
        assert!(matches!(refused(&[&good[0][..], &good[1][..]].concat(), "two frames"), Error::Decode(_)));
        assert!(matches!(refused(&[&good[0][..], &[0; 9][..]].concat(), "a frame and more"), Error::Decode(_)));

        // One bit of the samples flipped, and one of the header: each has a CRC.
        let mut damaged = good[0].clone();
        let middle = damaged.len() / 2;
        damaged[middle] ^= 1;
        assert!(matches!(refused(&damaged, "a bit of the samples flipped"), Error::Decode(_)));
        let mut damaged = good[0].clone();
        damaged[3] ^= 0x10;
        assert!(matches!(refused(&damaged, "a bit of the header flipped"), Error::Decode(_)));

        // Good frames of another shape: half the block, one channel, another
        // width and another rate.
        for other in [
            Stream { block: 480, ..STEREO },
            Stream { channels: 1, ..STEREO },
            Stream { bits: 8, ..STEREO },
            Stream { rate: 44_100, ..STEREO },
        ] {
            let frame = flacenc(other, &signal(other, 1), 0).remove(0);
            assert!(matches!(refused(&frame, "another shape"), Error::Shape { want: STEREO, .. }), "{other:?}");
        }

        out.clear();
        decoder.decode(&good[1], &mut out).unwrap();
        assert_eq!(out, &samples[STEREO.samples()..]);
    }
}
