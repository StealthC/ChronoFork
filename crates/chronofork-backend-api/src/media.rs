//! Backend-neutral media capture seam.
//!
//! A backend that can produce video frames or audio samples exposes them
//! through [`MediaSource`]. Host layers take data from that trait and never
//! learn a concrete backend type, so the capture path stays reusable by the
//! CLI, the service, transports and the GUI.
//!
//! Frames and chunks are canonical output: they are functions of emulated
//! state and the master-cycle timeline. A capture layer must not feed host time
//! or window state back into them, and identical input from identical state
//! must produce byte-identical frames with identical [`VideoFrame::checksum`]
//! values.
//!
//! [`Backend::media_mut`](crate::Backend::media_mut) defaults to `None`, so a
//! backend without media (for example the synthetic test backend) stays
//! unchanged and honestly advertises no rendering or audio capability.

use std::error::Error;
use std::fmt;

use chronofork_core::{StateHash, hash_bytes};

/// Bytes in one RGBA8 pixel.
pub const RGBA_PIXEL_LEN: usize = 4;

/// A complete deterministic video frame in RGBA8.
///
/// `rgba` holds `width * height` pixels in row-major order, each as red, green,
/// blue, alpha. The frame is canonical output, not host presentation state: its
/// dimensions and bytes are decided by the emulated video processor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFrame {
    /// Frame width in pixels.
    pub width: u16,
    /// Frame height in pixels.
    pub height: u16,
    /// Row-major RGBA8 pixels, exactly `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl VideoFrame {
    /// Validates and wraps a frame.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::FrameLengthMismatch`] when `rgba` is not exactly
    /// `width * height * 4` bytes and
    /// [`MediaError::FrameDimensionsOverflow`] when that product does not fit
    /// the host's address space.
    pub fn new(width: u16, height: u16, rgba: Vec<u8>) -> Result<Self, MediaError> {
        let expected = expected_frame_len(width, height)
            .ok_or(MediaError::FrameDimensionsOverflow { width, height })?;
        if rgba.len() != expected {
            return Err(MediaError::FrameLengthMismatch {
                expected,
                found: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Returns the SHA-256 digest of the frame's canonical bytes.
    ///
    /// The canonical byte sequence is the little-endian `width`, the
    /// little-endian `height` and then the RGBA payload. Formatting, paths and
    /// host state are never hashed, so the digest is stable across runs and
    /// machines.
    #[must_use]
    pub fn checksum(&self) -> StateHash {
        let mut canonical = Vec::with_capacity(4 + self.rgba.len());
        canonical.extend_from_slice(&self.width.to_le_bytes());
        canonical.extend_from_slice(&self.height.to_le_bytes());
        canonical.extend_from_slice(&self.rgba);
        hash_bytes(&canonical)
    }
}

/// Returns the required RGBA payload length, or `None` on overflow.
fn expected_frame_len(width: u16, height: u16) -> Option<usize> {
    usize::from(width)
        .checked_mul(usize::from(height))?
        .checked_mul(RGBA_PIXEL_LEN)
}

/// A chunk of interleaved signed 16-bit audio samples.
///
/// `samples` are interleaved per frame: `channels` consecutive samples form one
/// sample frame, so the sample count must be a multiple of the channel count.
/// The sample rate is metadata for a consumer's playback policy; it is not
/// emulated time and never enters a snapshot or state hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioChunk {
    /// Interleaved signed 16-bit samples.
    pub samples: Vec<i16>,
    /// Sample frames per second.
    pub sample_rate_hz: u32,
    /// Interleaved channel count; must be non-zero.
    pub channels: u8,
}

impl AudioChunk {
    /// Validates and wraps an audio chunk.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::AudioChannelCountZero`] for a zero channel count
    /// (which would also make sample-frame alignment meaningless) and
    /// [`MediaError::AudioSampleCountNotAligned`] when the sample count is not
    /// a multiple of the channel count.
    pub fn new(samples: Vec<i16>, sample_rate_hz: u32, channels: u8) -> Result<Self, MediaError> {
        if channels == 0 {
            return Err(MediaError::AudioChannelCountZero);
        }
        let channel_count = usize::from(channels);
        if samples.len() % channel_count != 0 {
            return Err(MediaError::AudioSampleCountNotAligned {
                samples: samples.len(),
                channels,
            });
        }
        Ok(Self {
            samples,
            sample_rate_hz,
            channels,
        })
    }
}

/// Produces media from a backend as it executes.
///
/// `take_*` removes the item from the backend's bounded queue; a second call
/// before new media exists returns `None`. A capture layer must not assume more
/// than the backend documents.
pub trait MediaSource {
    /// Takes the next completed video frame, if one is available.
    fn take_video_frame(&mut self) -> Option<VideoFrame>;

    /// Takes the next completed audio chunk, if one is available.
    fn take_audio_chunk(&mut self) -> Option<AudioChunk>;
}

/// A malformed media item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaError {
    /// A frame payload length is not `width * height * 4`.
    FrameLengthMismatch {
        /// Required byte length.
        expected: usize,
        /// Supplied byte length.
        found: usize,
    },
    /// A frame's dimensions overflow the host's address space.
    FrameDimensionsOverflow {
        /// Supplied width in pixels.
        width: u16,
        /// Supplied height in pixels.
        height: u16,
    },
    /// An audio chunk declares zero channels.
    AudioChannelCountZero,
    /// An audio chunk's sample count is not a multiple of its channels.
    AudioSampleCountNotAligned {
        /// Supplied sample count.
        samples: usize,
        /// Supplied channel count.
        channels: u8,
    },
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameLengthMismatch { expected, found } => {
                write!(f, "video frame is {found} RGBA bytes, expected {expected}")
            }
            Self::FrameDimensionsOverflow { width, height } => write!(
                f,
                "video frame dimensions {width}x{height} overflow the address space"
            ),
            Self::AudioChannelCountZero => f.write_str("audio chunk declares zero channels"),
            Self::AudioSampleCountNotAligned { samples, channels } => write!(
                f,
                "audio chunk has {samples} samples, not a multiple of {channels} channels"
            ),
        }
    }
}

impl Error for MediaError {}
