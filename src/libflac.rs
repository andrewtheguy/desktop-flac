//! libFLAC, loaded at run time: the calls the encoder and the decoder make, and
//! where the library is found.
//!
//! [`api`] loads the system's shared library the first time a stream is set up,
//! unless [`load_from`] loaded the one an application carries: FLAC 1.5 or 1.4,
//! whose libraries are versions 14 and 12, the two the systems this is built
//! for have. The calls below are the whole interface, typed the same in both.
//! An encoder and a decoder are pointers their calls take; the one structure
//! read is the head of a decoded frame's header ([`FrameHeader`]).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;

use crate::Error;

/// `FLAC__StreamEncoderWriteCallback`: `bytes` of the stream at `buffer`, which
/// are a frame when `samples` is the block it holds and metadata when it is
/// zero.
pub type EncoderWrite = unsafe extern "C" fn(
    encoder: *const c_void,
    buffer: *const u8,
    bytes: usize,
    samples: u32,
    frame: u32,
    client: *mut c_void,
) -> c_int;

/// `FLAC__StreamDecoderReadCallback`: fill `buffer` with up to `*bytes` of the
/// stream and say in `*bytes` how many there were.
pub type DecoderRead =
    unsafe extern "C" fn(decoder: *const c_void, buffer: *mut u8, bytes: *mut usize, client: *mut c_void) -> c_int;

/// `FLAC__StreamDecoderTellCallback`: say in `*offset` how many bytes of the
/// stream have been read.
pub type DecoderTell = unsafe extern "C" fn(decoder: *const c_void, offset: *mut u64, client: *mut c_void) -> c_int;

/// `FLAC__StreamDecoderWriteCallback`: one decoded frame, a pointer to each
/// channel's samples in `buffer`.
pub type DecoderWrite = unsafe extern "C" fn(
    decoder: *const c_void,
    frame: *const FrameHeader,
    buffer: *const *const i32,
    client: *mut c_void,
) -> c_int;

/// `FLAC__StreamDecoderErrorCallback`: something in the stream was not FLAC.
pub type DecoderError = unsafe extern "C" fn(decoder: *const c_void, status: c_int, client: *mut c_void);

/// `FLAC__STREAM_DECODER_READ_STATUS_CONTINUE`.
pub const READ_CONTINUE: c_int = 0;
/// `FLAC__STREAM_DECODER_READ_STATUS_END_OF_STREAM`.
pub const READ_END: c_int = 1;
/// `FLAC__STREAM_DECODER_TELL_STATUS_OK`.
pub const TELL_OK: c_int = 0;
/// `FLAC__STREAM_DECODER_WRITE_STATUS_CONTINUE`, and the encoder's
/// `FLAC__STREAM_ENCODER_WRITE_STATUS_OK`.
pub const WRITE_CONTINUE: c_int = 0;
/// `FLAC__STREAM_DECODER_WRITE_STATUS_ABORT`.
pub const WRITE_ABORT: c_int = 1;

/// The head of `FLAC__FrameHeader`, which a `FLAC__Frame` starts with: the same
/// in FLAC 1.4 and 1.5.
#[repr(C)]
pub struct FrameHeader {
    pub blocksize: u32,
    pub sample_rate: u32,
    pub channels: u32,
    _channel_assignment: c_int,
    pub bits_per_sample: u32,
}

/// Declares the calls once: the table's fields, and the names looked up in the
/// loaded library, each `FLAC__stream_` and the field's own.
macro_rules! calls {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {
        pub struct Api {
            $(pub $name: unsafe extern "C" fn($($ty),*) $(-> $ret)?,)*
            /// Keeps the loaded library mapped for as long as the pointers above
            /// live.
            library: libloading::Library,
        }

        fn resolve(library: libloading::Library) -> Result<Api, libloading::Error> {
            // SAFETY: each symbol is typed as libFLAC's headers declare it, the
            // same in every release loaded, and the library is kept in the table
            // the pointers are copied into.
            let api = unsafe {
                Api {
                    $($name: *library.get(concat!("FLAC__stream_", stringify!($name), "\0").as_bytes())?,)*
                    library,
                }
            };
            // Moving the library into the table leaves the pointers as they were.
            Ok(api)
        }
    };
}

// A `FLAC__bool` is an `int`, zero for false. A callback a stream does without
// is null, so those are untyped.
calls! {
    fn encoder_new() -> *mut c_void;
    fn encoder_delete(encoder: *mut c_void);
    fn encoder_set_streamable_subset(encoder: *mut c_void, value: c_int) -> c_int;
    fn encoder_set_do_md5(encoder: *mut c_void, value: c_int) -> c_int;
    fn encoder_set_channels(encoder: *mut c_void, value: u32) -> c_int;
    fn encoder_set_bits_per_sample(encoder: *mut c_void, value: u32) -> c_int;
    fn encoder_set_sample_rate(encoder: *mut c_void, value: u32) -> c_int;
    fn encoder_set_blocksize(encoder: *mut c_void, value: u32) -> c_int;
    fn encoder_init_stream(
        encoder: *mut c_void,
        write: EncoderWrite,
        seek: *const c_void,
        tell: *const c_void,
        metadata: *const c_void,
        client: *mut c_void
    ) -> c_int;
    fn encoder_process_interleaved(encoder: *mut c_void, samples: *const i32, frames: u32) -> c_int;
    fn encoder_finish(encoder: *mut c_void) -> c_int;
    fn encoder_get_resolved_state_string(encoder: *const c_void) -> *const c_char;
    fn decoder_new() -> *mut c_void;
    fn decoder_delete(decoder: *mut c_void);
    fn decoder_init_stream(
        decoder: *mut c_void,
        read: DecoderRead,
        seek: *const c_void,
        tell: DecoderTell,
        length: *const c_void,
        eof: *const c_void,
        write: DecoderWrite,
        metadata: *const c_void,
        error: DecoderError,
        client: *mut c_void
    ) -> c_int;
    fn decoder_process_until_end_of_stream(decoder: *mut c_void) -> c_int;
    fn decoder_get_decode_position(decoder: *const c_void, position: *mut u64) -> c_int;
    fn decoder_finish(decoder: *mut c_void) -> c_int;
    fn decoder_get_resolved_state_string(decoder: *const c_void) -> *const c_char;
}

/// The files tried, newest first: FLAC 1.5's library and 1.4's. A Windows
/// build's has no version in its name.
const FILES: &[&str] = if cfg!(target_os = "macos") {
    &["libFLAC.14.dylib", "libFLAC.12.dylib"]
} else if cfg!(windows) {
    &["libFLAC.dll"]
} else {
    &["libFLAC.so.14", "libFLAC.so.12"]
};

/// Where the system's library is looked for, in order. The empty folder is the
/// platform loader's own search, which on Windows starts beside the
/// executable; the others are where Homebrew and MacPorts install FLAC, which
/// that search does not reach.
const DIRS: &[&str] =
    if cfg!(target_os = "macos") { &["", "/opt/homebrew/lib", "/usr/local/lib", "/opt/local/lib"] } else { &[""] };

/// How to get what [`FILES`] names, for the error that says it is missing.
const INSTALL: &str = if cfg!(target_os = "macos") {
    "install it with `brew install flac`"
} else if cfg!(windows) {
    "put the libFLAC.dll of a FLAC release beside the executable or on PATH"
} else {
    "install your distribution's libFLAC, libflac14 or libflac12"
};

/// libFLAC, once loaded.
static API: OnceLock<Api> = OnceLock::new();

/// libFLAC, loaded from the system on the first call that finds it, unless
/// [`load_from`] loaded it before. A failure is not remembered, so a library
/// installed while the process runs is found by the next stream.
pub fn api() -> Result<&'static Api, Error> {
    if let Some(api) = API.get() {
        return Ok(api);
    }
    match find(DIRS) {
        Ok(api) => Ok(API.get_or_init(|| api)),
        Err(tried) => Err(Error::Missing { install: INSTALL.into(), tried }),
    }
}

/// Load the system's libFLAC now, so a process finds out when it starts, and
/// not at its first stream, that there is none.
pub fn load() -> Result<(), Error> {
    api().map(|_| ())
}

/// Load libFLAC from `dir`, the folder an application carries its own in, and
/// from nowhere else: an application that brought one does not code with
/// another it happens to find. Called before any stream is set up.
pub fn load_from(dir: &Path) -> Result<(), Error> {
    match find(&[dir]) {
        // One already loaded would be the one every stream uses, so the named
        // folder's is not dropped in silence for it.
        Ok(api) => API.set(api).map_err(|_| Error::AlreadyLoaded(dir.to_path_buf())),
        Err(tried) => Err(Error::Missing { install: format!("{} should hold it", dir.display()), tried }),
    }
}

/// The first libFLAC in `dirs` that loads and has every call, or why each file
/// tried was refused.
fn find<P: AsRef<Path>>(dirs: &[P]) -> Result<Api, String> {
    let mut refused = Vec::new();
    for dir in dirs {
        for file in FILES {
            let path = dir.as_ref().join(file);
            let file = path.display();
            // SAFETY: libFLAC's initialisers set up nothing but its own tables.
            match unsafe { libloading::Library::new(&path) }.and_then(resolve) {
                Ok(api) => return Ok(api),
                // The system's own reason is under libloading's.
                Err(e) => refused.push(match std::error::Error::source(&e) {
                    Some(system) => format!("{file}: {e}: {system}"),
                    None => format!("{file}: {e}"),
                }),
            }
        }
    }
    Err(refused.join("; "))
}

impl Api {
    /// The name libFLAC's array `symbol` has for `status`, one of its first
    /// `known`: the statuses both releases name.
    fn status(&self, symbol: &[u8], known: c_int, status: c_int) -> String {
        // SAFETY: the symbol is libFLAC's array of a static name for each status
        // from zero, at least `known` long in both releases, and only one of
        // those is read.
        unsafe {
            match self.library.get::<*const *const c_char>(symbol) {
                Ok(names) if (0..known).contains(&status) => {
                    CStr::from_ptr(*names.add(status as usize)).to_string_lossy().into_owned()
                }
                _ => format!("status {status}"),
            }
        }
    }

    /// What libFLAC calls the status an encoder's init returned.
    pub fn encoder_init_status(&self, status: c_int) -> String {
        self.status(b"FLAC__StreamEncoderInitStatusString\0", 14, status)
    }

    /// What libFLAC calls the status a decoder's init returned.
    pub fn decoder_init_status(&self, status: c_int) -> String {
        self.status(b"FLAC__StreamDecoderInitStatusString\0", 6, status)
    }

    /// What libFLAC calls the status its decoder reported an error with.
    pub fn decoder_error_status(&self, status: c_int) -> String {
        self.status(b"FLAC__StreamDecoderErrorStatusString\0", 5, status)
    }
}

/// libFLAC's own static string for a state, as text.
///
/// # Safety
///
/// `state` is what one of libFLAC's `get_resolved_state_string` calls returned.
pub unsafe fn state(state: *const c_char) -> String {
    // SAFETY: a static NUL-terminated string of libFLAC's.
    unsafe { CStr::from_ptr(state) }.to_string_lossy().into_owned()
}
