//! # audio-host
//!
//! A backend-agnostic Rust library for managing audio input and output devices. This crate provides a unified, high-level interface for interacting with various audio backends, allowing you to write audio application code that is independent of the underlying audio driver implementation.
//!
//! ## Features
//!
//! `audio-host` abstracts over several popular audio backends. You can choose the one that best fits your needs via Cargo features.
//!
//! **Available Backends:**
//!
//! - **`rtaudio`** (Default): Uses the [RtAudio](https://codeberg.org/Meadowlark/rtaudio-rs) C++ library wrapper.
//! - **`juce`**: Uses [`cxx-juce`](https://github.com/JamesHallowell/cxx-juce) to interface with the JUCE C++ framework.
//! - **`cpal`**: Uses the [Cross-Platform Audio Library](https://github.com/RustAudio/cpal) (pure Rust).
//!
//! ### ⚠️ Important: Mutual Exclusivity
//!
//! **You must enable exactly one audio backend feature at a time.**
//!
//! These backends are mutually exclusive. Trying to enable more than one (e.g., `cpal` and `juce` together) will result in a compile-time error.
//!
//! ## Installation
//!
//! Add `audio-host` to your `Cargo.toml`.
//!
//! To use the default backend (`rtaudio`):
//!
//! ```toml
//! [dependencies]
//! audio-host = "0.2.0"
//! ```
//!
//! To use a specific backend (e.g., `cpal`), disable the default features:
//!
//! ```toml
//! [dependencies]
//! audio-host = { version = "0.2.0", default-features = false, features = ["cpal"] }
//! ```
//!
//! ## Usage
//!
//! ### Listing devices
//!
//! ```rust,no_run
//! use audio_host::{AudioHost, Error, AudioBackend};
//!
//! fn main() -> Result<(), Error> {
//!     let host = AudioHost::new()?;
//!
//!     println!("API:     {}", host.api());
//!     println!("APIs:    {:#?}", host.apis());
//!     println!("Inputs:  {:#?}", host.inputs());
//!     println!("Outputs: {:#?}", host.outputs());
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Selecting devices
//!
//! Call `set_api`, `set_input`, or `set_output` with the full name or a part of it before starting the stream. An exact match is preferred, otherwise the first name containing the text is used. Each returns `Err(Error::NotFound)` if no matching device is found.
//!
//! ```rust,no_run
//! use audio_host::{AudioHost, Error, AudioBackend, Config};
//!
//! fn main() -> Result<(), Error> {
//!     let mut host = AudioHost::new()?;
//!
//!     host.set_api("ALSA")?;
//!     host.set_input("Focusrite")?;
//!     host.set_output("Focusrite")?;
//!
//!     // ...
//!     Ok(())
//! }
//! ```
//!
//! > **Note:** Some backends (e.g. cpal on Linux) expose a virtual "Default Audio Device" as the
//! > default that does not appear in the `inputs()` / `outputs()` lists. In that case, omit the
//! > `set_input` / `set_output` calls and rely on the default selected by `AudioHost::new()`.
//!
//! ### Starting a stream
//!
//! ```rust,no_run
//! use audio_host::{AudioBlockOpsMut, AudioHost, Error, AudioBackend, Config};
//!
//! fn main() -> Result<(), Error> {
//!     let mut host = AudioHost::new()?;
//!
//!     host.start(
//!         Config::default()
//!             .with_input_channels(2)
//!             .with_output_channels(2)
//!             .with_sample_rate(48000)
//!             .with_num_frames(1024),
//!         move |input, mut output| {
//!             // Simple pass-through: copy input to output
//!             output.copy_from_block(&input);
//!         },
//!     )?;
//!
//!     std::thread::sleep(std::time::Duration::from_secs(5));
//!
//!     host.stop()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! Set `num_input_channels` or `num_output_channels` to `0` to open an output-only or input-only stream.
//!
//! If `start` succeeds, the stream runs with exactly the requested channel counts and sample
//! rate. Otherwise it returns `Error::UnsupportedInputChannels`,
//! `Error::UnsupportedOutputChannels` or `Error::UnsupportedSampleRate` with what the
//! device supports, so you can retry with a supported config. `num_frames` is only a request:
//! the process callback can receive blocks of a different size. It can be at most
//! `MAX_NUM_FRAMES` (32768).
//!
//! ### Monitoring the stream
//!
//! Poll `status()` from your own thread, e.g. once per UI frame. It reports whether the stream
//! is still running, why it stopped on its own (e.g. the device was unplugged) and how many
//! dropouts (xruns) happened. Streams aren't restarted automatically: call `start` again or
//! pick another device.
//!
//! ```rust,no_run
//! use audio_host::{AudioBackend, AudioHost, Config, Error};
//!
//! fn main() -> Result<(), Error> {
//!     let mut host = AudioHost::new()?;
//!     host.start(Config::default(), |_input, _output| {})?;
//!
//!     loop {
//!         std::thread::sleep(std::time::Duration::from_millis(100));
//!         let status = host.status();
//!         if !status.running {
//!             eprintln!("stream stopped: {:?}", status.error);
//!             break;
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ### Threads
//!
//! `AudioHost` is only `Send` on the `cpal` backend: rtaudio and JUCE must stay on the thread
//! that created them. For code that works with every backend, create and use the host on one
//! thread and talk to it through a channel. The process callback is `Send` on every backend,
//! as it runs on the audio thread.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

mod compile_error;

#[cfg(feature = "cpal")]
pub mod backend_cpal;
#[cfg(feature = "juce")]
pub mod backend_juce;
#[cfg(feature = "rtaudio")]
pub mod backend_rtaudio;

#[cfg(feature = "cpal")]
pub use backend_cpal::AudioHost;
#[cfg(feature = "juce")]
pub use backend_juce::AudioHost;
#[cfg(feature = "rtaudio")]
pub use backend_rtaudio::AudioHost;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("Device or API not found")]
    NotFound,
    #[error("Invalid config: {0}")]
    InvalidConfig(&'static str),
    /// The input device cannot open the requested number of channels.
    /// `supported` is the closest channel count it can open, so retrying with it works
    /// (`0` if there is no input device).
    #[error("{}", channel_message("Input", *.requested, *.supported))]
    UnsupportedInputChannels { requested: u16, supported: u16 },
    /// The output device cannot open the requested number of channels.
    /// `supported` is the closest channel count it can open, so retrying with it works
    /// (`0` if there is no output device).
    #[error("{}", channel_message("Output", *.requested, *.supported))]
    UnsupportedOutputChannels { requested: u16, supported: u16 },
    /// The devices cannot run at the requested sample rate.
    /// `available` lists the sample rates they support (may be empty if unknown).
    #[error("Sample rate {requested} Hz is not supported, available: {available:?}")]
    UnsupportedSampleRate { requested: u32, available: Vec<u32> },
    #[error("Backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Finds the item whose name matches `query`: an exact match wins, otherwise the first name
/// that contains `query`. Items without a name are skipped.
pub(crate) fn find_by_name<T>(
    items: impl IntoIterator<Item = T>,
    name: impl Fn(&T) -> Option<String>,
    query: &str,
) -> Option<T> {
    let mut partial = None;
    for item in items {
        match name(&item) {
            Some(n) if n == query => return Some(item),
            Some(n) if partial.is_none() && n.contains(query) => partial = Some(item),
            _ => {}
        }
    }
    partial
}

fn channel_message(direction: &str, requested: u16, supported: u16) -> String {
    if supported == 0 {
        format!("No {} device available", direction.to_lowercase())
    } else {
        format!("{direction} device can open {supported} channels, but {requested} were requested")
    }
}

pub type Block<'a> = InterleavedView<'a, f32>;
pub type BlockMut<'a> = InterleavedViewMut<'a, f32>;

pub use audio_blocks::*;

/// A device listed by [`AudioBackend::inputs`] or [`AudioBackend::outputs`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct DeviceInfo {
    pub name: String,
    /// The most channels the device can open, in the direction it was listed for
    /// (input or output). `None` if the backend can't tell without opening the device (JUCE).
    pub num_channels: Option<u16>,
}

/// The largest `num_frames` [`Config::validate`] accepts (about 680 ms at 48 kHz). Devices
/// support far smaller buffers, and `num_frames` is only a request anyway.
pub const MAX_NUM_FRAMES: usize = 32768;

/// The stream configuration passed to [`AudioBackend::start`].
///
/// Build it from [`Config::default`] with the `with_*` methods, so fields can be added later
/// without a breaking change:
///
/// ```
/// use audio_host::Config;
///
/// let config = Config::default()
///     .with_input_channels(2)
///     .with_output_channels(2)
///     .with_sample_rate(44100)
///     .with_num_frames(256);
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Config {
    /// `0` for an output-only stream.
    pub num_input_channels: u16,
    /// `0` for an input-only stream.
    pub num_output_channels: u16,
    pub sample_rate: u32,
    /// The requested block size, at most [`MAX_NUM_FRAMES`]. Only a request: the process
    /// callback can receive blocks of a different size.
    pub num_frames: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num_input_channels: 0,
            num_output_channels: 2,
            sample_rate: 48000,
            num_frames: 512,
        }
    }
}

impl Config {
    #[must_use]
    pub const fn with_input_channels(mut self, num_input_channels: u16) -> Self {
        self.num_input_channels = num_input_channels;
        self
    }

    #[must_use]
    pub const fn with_output_channels(mut self, num_output_channels: u16) -> Self {
        self.num_output_channels = num_output_channels;
        self
    }

    #[must_use]
    pub const fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    #[must_use]
    pub const fn with_num_frames(mut self, num_frames: usize) -> Self {
        self.num_frames = num_frames;
        self
    }

    /// Checks the config without opening a device. [`AudioBackend::start`] calls this first.
    pub fn validate(&self) -> Result<(), Error> {
        if self.num_input_channels == 0 && self.num_output_channels == 0 {
            return Err(Error::InvalidConfig(
                "at least one of num_input_channels or num_output_channels must be > 0",
            ));
        }
        if self.sample_rate == 0 {
            return Err(Error::InvalidConfig("sample_rate must be > 0"));
        }
        if self.num_frames == 0 {
            return Err(Error::InvalidConfig("num_frames must be > 0"));
        }
        if self.num_frames > MAX_NUM_FRAMES {
            // keep in sync with MAX_NUM_FRAMES (checked by a test)
            return Err(Error::InvalidConfig("num_frames must be <= 32768"));
        }
        Ok(())
    }
}

/// Why a stream stopped on its own.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StreamError {
    /// The device was disconnected or is no longer available.
    #[error("Audio device lost")]
    DeviceLost,
    /// The backend stopped the stream, with its description of why.
    #[error("Stream stopped by the backend: {0}")]
    Backend(String),
}

/// The state of the stream, see [`AudioBackend::status`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StreamStatus {
    /// The stream was started and hasn't been stopped by [`AudioBackend::stop`] or an error.
    pub running: bool,
    /// Why the stream stopped on its own, if it did. Cleared by the next `start`.
    pub error: Option<StreamError>,
    /// Dropouts since `start`: callbacks where audio was lost or replaced by silence
    /// because the system couldn't keep up. Always `0` on JUCE, which doesn't report them.
    pub xruns: u64,
}

/// Collects the stream status from the backend's audio and error callbacks.
/// Counting xruns is real-time safe; errors take a lock, but they are rare.
#[derive(Debug, Default)]
pub(crate) struct StatusTracker {
    running: AtomicBool,
    xruns: AtomicU64,
    error: Mutex<Option<StreamError>>,
}

#[allow(dead_code)] // not every backend reports everything (e.g. JUCE has no xruns)
impl StatusTracker {
    /// Clears errors and xruns before a new stream starts.
    pub(crate) fn reset(&self) {
        let mut error = self.error.lock().unwrap_or_else(|e| e.into_inner());
        self.running.store(false, Ordering::Relaxed);
        self.xruns.store(0, Ordering::Relaxed);
        *error = None;
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Real-time safe.
    pub(crate) fn add_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }

    /// Marks a running stream as stopped by `error`. The first error is kept.
    pub(crate) fn fail(&self, error: StreamError) {
        // already stopped: skip the lock, so repeated errors don't take it on the audio thread
        if !self.is_running() {
            return;
        }
        // `running` changes under the error lock, so `status` never sees a stopped stream
        // whose error isn't stored yet
        let mut current = self.error.lock().unwrap_or_else(|e| e.into_inner());
        if !self.running.swap(false, Ordering::Relaxed) {
            return;
        }
        if current.is_none() {
            *current = Some(error);
        }
    }

    /// Marks the stream as running again if it was stopped by `error`, e.g. when the backend
    /// restarted the device on its own. Does nothing after `stop` or other errors.
    pub(crate) fn resume_after(&self, error: &StreamError) {
        let mut current = self.error.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_ref() == Some(error) {
            *current = None;
            self.running.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn status(&self) -> StreamStatus {
        let error = self.error.lock().unwrap_or_else(|e| e.into_inner());
        StreamStatus {
            running: self.is_running(),
            error: error.clone(),
            xruns: self.xruns.load(Ordering::Relaxed),
        }
    }
}

/// Trait defining the common interface for audio devices.
///
/// `AudioHost` is only `Send` on the `cpal` backend, see [Threads](crate#threads).
pub trait AudioBackend {
    /// Create a new audio device with default settings
    fn new() -> Result<Self, Error>
    where
        Self: Sized;

    /// Get the current API/host name
    fn api(&self) -> String;

    /// Get all available APIs/hosts
    fn apis(&self) -> Vec<String>;

    /// Get the current input device name, empty if there is none.
    ///
    /// JUCE only knows its default device after `start`, so this is empty until then if no
    /// device was selected with `set_input`.
    fn input(&self) -> String;

    /// Get the current output device name, empty if there is none.
    ///
    /// JUCE only knows its default device after `start`, so this is empty until then if no
    /// device was selected with `set_output`.
    fn output(&self) -> String;

    /// Get all available input devices
    fn inputs(&self) -> Vec<DeviceInfo>;

    /// Get all available output devices
    fn outputs(&self) -> Vec<DeviceInfo>;

    /// Set the API/host by name
    fn set_api(&mut self, name: &str) -> Result<(), Error>;

    /// Set the input device by name
    fn set_input(&mut self, input: &str) -> Result<(), Error>;

    /// Set the output device by name
    fn set_output(&mut self, output: &str) -> Result<(), Error>;

    /// Start the audio stream with the given configuration and process callback.
    ///
    /// On success the stream uses exactly the requested channel counts and sample rate.
    /// Otherwise an `Unsupported*` error reports what the device supports.
    ///
    /// `process_fn` gets the input and output of each callback, with the same number of
    /// frames. In an input-only or output-only stream, the unused block is empty (0 channels,
    /// 0 frames).
    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error>;

    /// Stop the audio stream
    fn stop(&mut self) -> Result<(), Error>;

    /// The state of the stream: whether it is still running, why it stopped on its own (e.g.
    /// the device was unplugged) and how many dropouts happened. Poll this from your own
    /// thread, e.g. once per UI frame. Streams aren't restarted automatically.
    fn status(&self) -> StreamStatus;
}

// Tests that need audio hardware are in `tests/hardware.rs`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_output_only() {
        let config = Config::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.num_input_channels, 0);
        assert_eq!(config.num_output_channels, 2);
    }

    #[test]
    fn input_only_and_output_only_configs_are_valid() {
        let input_only = Config::default()
            .with_input_channels(1)
            .with_output_channels(0);
        let output_only = Config::default()
            .with_input_channels(0)
            .with_output_channels(1);
        assert!(input_only.validate().is_ok());
        assert!(output_only.validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_configs() {
        let invalid = [
            Config::default()
                .with_input_channels(0)
                .with_output_channels(0),
            Config::default().with_sample_rate(0),
            Config::default().with_num_frames(0),
            Config::default().with_num_frames(MAX_NUM_FRAMES + 1),
            Config::default().with_num_frames(usize::MAX),
        ];
        for config in invalid {
            assert!(
                matches!(config.validate(), Err(Error::InvalidConfig(_))),
                "{config:?} should be invalid"
            );
        }
    }

    #[test]
    fn validate_accepts_the_largest_num_frames() {
        let config = Config::default().with_num_frames(MAX_NUM_FRAMES);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn num_frames_error_names_the_limit() {
        let Err(Error::InvalidConfig(message)) = Config::default()
            .with_num_frames(MAX_NUM_FRAMES + 1)
            .validate()
        else {
            panic!("expected InvalidConfig");
        };
        assert!(message.contains(&MAX_NUM_FRAMES.to_string()), "{message}");
    }

    #[test]
    fn builder_sets_every_field() {
        let config = Config::default()
            .with_input_channels(3)
            .with_output_channels(4)
            .with_sample_rate(44100)
            .with_num_frames(256);
        assert_eq!(config.num_input_channels, 3);
        assert_eq!(config.num_output_channels, 4);
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.num_frames, 256);
    }

    #[test]
    fn find_by_name_prefers_exact_match() {
        let names = ["USB Speakers", "Speakers (2)", "Speakers"];
        let find = |query| find_by_name(names, |n| Some(n.to_string()), query);
        assert_eq!(find("Speakers"), Some("Speakers"));
        assert_eq!(find("Speak"), Some("USB Speakers"));
        assert_eq!(find("(2)"), Some("Speakers (2)"));
        assert_eq!(find("Headphones"), None);
    }

    #[test]
    fn find_by_name_skips_unnamed_items() {
        let items = [None, Some("Speakers")];
        let found = find_by_name(items, |n| n.map(str::to_string), "Speakers");
        assert_eq!(found, Some(Some("Speakers")));
    }

    #[test]
    fn status_tracks_running_errors_and_xruns() {
        let status = StatusTracker::default();
        assert_eq!(status.status(), StreamStatus::default());

        status.set_running(true);
        status.add_xrun();
        status.add_xrun();
        status.fail(StreamError::DeviceLost);
        // the first error is kept
        status.fail(StreamError::Backend("later".into()));
        assert_eq!(
            status.status(),
            StreamStatus {
                running: false,
                error: Some(StreamError::DeviceLost),
                xruns: 2,
            }
        );

        status.reset();
        assert_eq!(status.status(), StreamStatus::default());
    }

    #[test]
    fn status_never_shows_a_stopped_stream_without_its_error() {
        for _ in 0..1000 {
            let status = StatusTracker::default();
            status.set_running(true);
            std::thread::scope(|s| {
                s.spawn(|| {
                    // poll like a caller would until the stream stops
                    loop {
                        let current = status.status();
                        if !current.running {
                            assert_eq!(current.error, Some(StreamError::DeviceLost));
                            break;
                        }
                    }
                });
                status.fail(StreamError::DeviceLost);
            });
        }
    }

    #[test]
    fn status_resumes_only_after_the_given_error() {
        let stopped = StreamError::Backend("stopped".into());
        let status = StatusTracker::default();

        // stopped by the user: stays stopped
        status.resume_after(&stopped);
        assert!(!status.status().running);

        // another error: stays failed
        status.set_running(true);
        status.fail(StreamError::DeviceLost);
        status.resume_after(&stopped);
        assert_eq!(status.status().error, Some(StreamError::DeviceLost));

        status.reset();
        status.set_running(true);
        status.fail(stopped.clone());
        status.resume_after(&stopped);
        assert_eq!(
            status.status(),
            StreamStatus {
                running: true,
                ..StreamStatus::default()
            }
        );
    }

    #[test]
    fn status_ignores_errors_when_not_running() {
        // e.g. errors while a stream is being stopped
        let status = StatusTracker::default();
        status.fail(StreamError::DeviceLost);
        assert_eq!(status.status().error, None);
    }

    #[test]
    fn channel_error_messages() {
        let error = Error::UnsupportedInputChannels {
            requested: 3,
            supported: 2,
        };
        assert_eq!(
            error.to_string(),
            "Input device can open 2 channels, but 3 were requested"
        );

        let error = Error::UnsupportedOutputChannels {
            requested: 2,
            supported: 0,
        };
        assert_eq!(error.to_string(), "No output device available");
    }
}
