# audio-device

A backend-agnostic Rust library for managing audio input and output devices. This crate provides a unified, high-level interface for interacting with various audio backends, allowing you to write audio application code that is independent of the underlying audio driver implementation.

## Features

`audio-device` abstracts over several popular audio backends. You can choose the one that best fits your needs via Cargo features.

**Available Backends:**

- **`juce`** (Default): Uses [`cxx-juce`](https://github.com/JamesHallowell/cxx-juce) to interface with the JUCE C++ framework.
- **`cpal`**: Uses the [Cross-Platform Audio Library](https://github.com/RustAudio/cpal) (pure Rust).
- **`rtaudio`**: Uses the [RtAudio](https://codeberg.org/Meadowlark/rtaudio-rs) C++ library wrapper.

### ⚠️ Important: Mutual Exclusivity

**You must enable exactly one audio backend feature at a time.**

These backends are mutually exclusive. Trying to enable more than one (e.g., `cpal` and `juce` together) will result in a compile-time error.

## Installation

Add `audio-device` to your `Cargo.toml`.

To use the default backend (`juce`):

```toml
[dependencies]
audio-device = "0.1.0"
```

To use a specific backend (e.g., `cpal`), disable the default features:

```toml
[dependencies]
audio-device = { version = "0.1.0", default-features = false, features = ["cpal"] }
```

## Usage

Regardless of the selected backend, the API remains consistent. Here is a basic example of how to list devices and start an audio stream.

```rust
use std::error::Error;
use audio_blocks::{AudioBlock, AudioBlockOps};
use audio_device::{AudioDevice, AudioDeviceTrait, Config};

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize the audio device (uses the backend selected via features)
    let mut device = AudioDevice::new()?;

    // Print available APIs and Devices
    println!("Current API: {}", device.api());
    println!("Available APIs: {:#?}", device.apis());
    println!("Input Devices: {:#?}", device.inputs());
    println!("Output Devices: {:#?}", device.outputs());

    // Configure the stream
    let config = Config {
        num_input_channels: 2,
        num_output_channels: 2,
        sample_rate: 48000,
        num_frames: 1024,
    };

    // Start the audio stream
    // The callback receives an input block (read-only) and an output block (mutable)
    device.start(config, move |input, mut output| {
        // Simple pass-through: copy input to output
        output.copy_from_block(&input);
    })?;

    // Keep the main thread alive while audio processes in the background
    std::thread::sleep(std::time::Duration::from_secs(5));

    // Stop the stream
    device.stop()?;

    Ok(())
}
```
