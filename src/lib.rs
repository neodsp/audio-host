//! # audio-io
//!
//! A backend-agnostic Rust library for managing audio input and output devices. This crate provides a unified, high-level interface for interacting with various audio backends, allowing you to write audio application code that is independent of the underlying audio driver implementation.
//!
//! ## Features
//!
//! `audio-io` abstracts over several popular audio backends. You can choose the one that best fits your needs via Cargo features.
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
//! Add `audio-io` to your `Cargo.toml`.
//!
//! To use the default backend (`rtaudio`):
//!
//! ```toml
//! [dependencies]
//! audio-io = "0.5.0"
//! ```
//!
//! To use a specific backend (e.g., `cpal`), disable the default features:
//!
//! ```toml
//! [dependencies]
//! audio-io = { version = "0.5.0", default-features = false, features = ["cpal"] }
//! ```
//!
//! ## Usage
//!
//! ### Listing devices
//!
//! ```rust
//! use audio_io::{AudioHost, Error, AudioBackend};
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
//! use audio_io::{AudioHost, Error, AudioBackend, Config};
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
//! ```rust
//! use audio_io::{AudioBlockOpsMut, AudioHost, Error, AudioBackend, Config};
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
pub enum Error {
    #[error("Device or API not found")]
    NotFound,
    #[error("Invalid config: {0}")]
    InvalidConfig(&'static str),
    #[error("Backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
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
            num_input_channels: 2,
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

    /// Start the audio stream with the given configuration and process callback
    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error>;

    /// Stop the audio stream
    fn stop(&mut self) -> Result<(), Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_device() {
        let mut host = AudioHost::new().unwrap();
        dbg!(host.apis());
        dbg!(host.inputs());
        dbg!(host.outputs());

        dbg!(host.api());
        dbg!(host.input());
        dbg!(host.output());

        host.set_api(&host.api()).unwrap();
        host.set_input(&host.input()).unwrap();
        host.set_output(&host.output()).unwrap();

        let num_frames = 1024;

        host.start(
            Config {
                num_input_channels: 2,
                num_output_channels: 2,
                sample_rate: 48000,
                num_frames,
            },
            move |input, mut output| {
                if output.copy_from_block(&input).is_some() {
                    eprintln!("Input and Output buffer did not have a similar size");
                }
            },
        )
        .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(3));

        host.stop().unwrap();
    }
}
