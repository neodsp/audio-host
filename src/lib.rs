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
//! Call `set_api`, `set_input`, or `set_output` with a substring of the desired name before starting the stream. Each returns `Err(Error::NotFound)` if no matching device is found.
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
//!         Config {
//!             num_input_channels: 2,
//!             num_output_channels: 2,
//!             sample_rate: 48000,
//!             num_frames: 1024,
//!         },
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
//! the process callback can receive blocks of a different size.

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

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeviceInfo {
    pub name: String,
    pub num_channels: u16,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    pub num_input_channels: u16,
    pub num_output_channels: u16,
    pub sample_rate: u32,
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
        Ok(())
    }
}

/// Trait defining the common interface for audio devices
pub trait AudioBackend {
    /// Create a new audio device with default settings
    fn new() -> Result<Self, Error>
    where
        Self: Sized;

    /// Get the current API/host name
    fn api(&self) -> String;

    /// Get all available APIs/hosts
    fn apis(&self) -> Vec<String>;

    /// Get the current input device name
    fn input(&self) -> String;

    /// Get the current output device name
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
    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error>;

    /// Stop the audio stream
    fn stop(&mut self) -> Result<(), Error>;
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
        let input_only = Config {
            num_input_channels: 1,
            num_output_channels: 0,
            ..Config::default()
        };
        let output_only = Config {
            num_input_channels: 0,
            num_output_channels: 1,
            ..Config::default()
        };
        assert!(input_only.validate().is_ok());
        assert!(output_only.validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_configs() {
        let invalid = [
            Config {
                num_input_channels: 0,
                num_output_channels: 0,
                ..Config::default()
            },
            Config {
                sample_rate: 0,
                ..Config::default()
            },
            Config {
                num_frames: 0,
                ..Config::default()
            },
        ];
        for config in invalid {
            assert!(
                matches!(config.validate(), Err(Error::InvalidConfig(_))),
                "{config:?} should be invalid"
            );
        }
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
