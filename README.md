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

### Optional CPAL features

These opt-in features enable `cpal` automatically and can be combined. Disable default
features to avoid also enabling RtAudio. The CPAL dependency is pinned to a GitHub revision.

| Feature | Purpose and requirements |
| --- | --- |
| `cpal-pipewire` | Native PipeWire on Linux/BSD; requires PipeWire development libraries and a running server. |
| `cpal-asio` | ASIO on Windows; requires an ASIO driver and LLVM/Clang for build-time bindings (see [CPAL's ASIO setup](https://github.com/RustAudio/cpal#asio-on-windows)). |
| `cpal-jack` | JACK on Linux/BSD, macOS, and Windows; requires JACK libraries and a running JACK-compatible server. |
| `cpal-pulseaudio` | Native PulseAudio on Linux/BSD; requires PulseAudio client libraries and a compatible server. |
| `cpal-realtime` | Real-time thread scheduling on supported Android/Linux/Windows hosts; Linux needs appropriate scheduling permissions. |
| `cpal-realtime-dbus` | Adds D-Bus/RTKit scheduling support on Linux; requires D-Bus development libraries and an available RTKit service. Includes `cpal-realtime`. |

ALSA on Linux, CoreAudio on macOS, and WASAPI on Windows need no extra CPAL feature.
Features enable host availability, not automatic host selection. Use `host.apis()` to
list available APIs and `host.set_api("PipeWire")`, `host.set_api("JACK")`, or
`host.set_api("ASIO")` before selecting devices and starting audio. Duplex support is
still detected per device; enabling a host feature does not guarantee a duplex stream.

For example, build with PipeWire and RTKit support:

```sh
cargo check --no-default-features --features cpal-pipewire,cpal-realtime-dbus
```

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

With the CPAL backend, selecting the same duplex-capable device for input and output
uses a single duplex callback, without the separate-stream input ring buffer or its
100 ms prefill. Both directions must be enabled. Different devices or devices without
native duplex support use separate streams. A duplex build/start failure is returned
as an error rather than retried with separate streams.

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
