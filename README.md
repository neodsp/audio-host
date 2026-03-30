<!-- cargo-rdme start -->

# audio-host

A backend-agnostic Rust library for managing audio input and output devices. This crate provides a unified, high-level interface for interacting with various audio backends, allowing you to write audio application code that is independent of the underlying audio driver implementation.

## Features

`audio-host` abstracts over several popular audio backends. You can choose the one that best fits your needs via Cargo features.

**Available Backends:**

- **`rtaudio`** (Default): Uses the [RtAudio](https://codeberg.org/Meadowlark/rtaudio-rs) C++ library wrapper.
- **`juce`**: Uses [`cxx-juce`](https://github.com/JamesHallowell/cxx-juce) to interface with the JUCE C++ framework.
- **`cpal`**: Uses the [Cross-Platform Audio Library](https://github.com/RustAudio/cpal) (pure Rust).

### ⚠️ Important: Mutual Exclusivity

**You must enable exactly one audio backend feature at a time.**

These backends are mutually exclusive. Trying to enable more than one (e.g., `cpal` and `juce` together) will result in a compile-time error.

## Installation

Add `audio-host` to your `Cargo.toml`.

To use the default backend (`rtaudio`):

```toml
[dependencies]
audio-host = "0.1.0"
```

To use a specific backend (e.g., `cpal`), disable the default features:

```toml
[dependencies]
audio-host = { version = "0.1.0", default-features = false, features = ["cpal"] }
```

## Usage

### Listing devices

```rust
use audio_host::{AudioHost, Error, AudioBackend};

fn main() -> Result<(), Error> {
    let host = AudioHost::new()?;

    println!("API:     {}", host.api());
    println!("APIs:    {:#?}", host.apis());
    println!("Inputs:  {:#?}", host.inputs());
    println!("Outputs: {:#?}", host.outputs());

    Ok(())
}
```

### Selecting devices

Call `set_api`, `set_input`, or `set_output` with a substring of the desired name before starting the stream. Each returns `Err(Error::NotFound)` if no matching device is found.

```rust
use audio_host::{AudioHost, Error, AudioBackend, Config};

fn main() -> Result<(), Error> {
    let mut host = AudioHost::new()?;

    host.set_api("ALSA")?;
    host.set_input("Focusrite")?;
    host.set_output("Focusrite")?;

    // ...
    Ok(())
}
```

> **Note:** Some backends (e.g. cpal on Linux) expose a virtual "Default Audio Device" as the
> default that does not appear in the `inputs()` / `outputs()` lists. In that case, omit the
> `set_input` / `set_output` calls and rely on the default selected by `AudioHost::new()`.

### Starting a stream

```rust
use audio_host::{AudioBlockOpsMut, AudioHost, Error, AudioBackend, Config};

fn main() -> Result<(), Error> {
    let mut host = AudioHost::new()?;

    host.start(
        Config {
            num_input_channels: 2,
            num_output_channels: 2,
            sample_rate: 48000,
            num_frames: 1024,
        },
        move |input, mut output| {
            // Simple pass-through: copy input to output
            output.copy_from_block(&input);
        },
    )?;

    std::thread::sleep(std::time::Duration::from_secs(5));

    host.stop()?;

    Ok(())
}
```

Set `num_input_channels` or `num_output_channels` to `0` to open an output-only or input-only stream.

<!-- cargo-rdme end -->
