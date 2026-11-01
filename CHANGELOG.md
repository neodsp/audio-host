# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-01

### Breaking changes

- `start()` is now strict: if it succeeds, the stream runs with exactly the requested channel
  counts and sample rate. Otherwise it returns the new `Error::UnsupportedInputChannels`,
  `Error::UnsupportedOutputChannels` or `Error::UnsupportedSampleRate` with what the device
  supports, so the config can be retried. A missing device counts as 0 supported channels.
  `num_frames` stays a request.
- `Config::default()` now requests 0 input and 2 output channels (was 2 and 2), so it works on
  machines without an input device.
- `DeviceInfo::num_channels` is now `Option<u16>` and is the most channels the device can
  open. It is `None` on the juce backend, which can't tell without opening the device.
- `Error`, `DeviceInfo`, `Config` and the new `StreamStatus` are `#[non_exhaustive]`, so
  fields can be added later without a breaking change. `DeviceInfo` and `Config` can no
  longer be built with a struct literal outside this crate. Build a `Config` with
  `Config::default().with_input_channels(2).with_output_channels(2)` and so on, or change
  its fields on a mutable value.
- `Config::validate()` (and so `start()`) rejects `num_frames` above the new `MAX_NUM_FRAMES`
  (32768) with `Error::InvalidConfig`.
- `AudioBackend` has a new required method, `status()`.
- `set_api`, `set_input` and `set_output` now prefer an exact name match over the first name
  containing the text.
- juce: `input()` and `output()` are empty until `start()` if no device was selected, because
  JUCE only knows its default devices after opening them.
- juce: `set_api` returns `Error::NotFound` for unknown APIs and APIs without devices (it
  used to return `Ok`).
- cpal, rtaudio: `set_api` now stops a running stream, like the juce backend.
- `Error::Backend` no longer returns the backend's error from `source()`. It is still part of
  the message, which error reporters printed twice before. Match on the variant to inspect it.
- `set_api`, `set_input` and `set_output` return `Error::NotFound` for an empty name, instead
  of selecting the first one.

### Added

- `Config::with_input_channels`, `with_output_channels`, `with_sample_rate` and
  `with_num_frames` builder methods.
- `Config::duplex_latency_ms` (and `with_duplex_latency_ms`) sets the input latency the cpal
  backend adds to duplex streams. It was fixed at 100 ms, which is still the default, and can
  be at most the new `MAX_DUPLEX_LATENCY_MS` (1000).
- `cpal-asio`, `cpal-jack`, `cpal-pipewire` and `cpal-pulseaudio` features add cpal's ASIO,
  JACK, PipeWire and PulseAudio hosts, chosen at runtime with `set_api`. Each one enables
  `cpal` and they can be combined. On Linux, cpal picks PipeWire or PulseAudio as the default
  host when it is enabled and running.
- `status()` reports whether the stream is still running, why it stopped on its own
  (`StreamError::DeviceLost` or `StreamError::Backend`) and how many xruns happened. On cpal,
  errors that some hosts survive are only reported if the stream then stops calling back.
- The cpal backend now supports input-only streams.
- With the `serde` feature, a `Config` with missing fields deserializes with their default
  values, so configs saved now still load after fields are added.

### Fixed

- The unused block in input-only and output-only streams now has 0 channels and 0 frames on
  every backend (cpal and rtaudio passed 1 channel, juce passed the callback's frame count).
- juce: the devices selected with `set_input` / `set_output` were ignored; JUCE always opened
  its default devices.
- juce: devices are only opened by `start()` and closed by `stop()`, and only the ones the
  stream uses. `new()` no longer fails when a default device is busy, and output-only streams
  no longer trigger the microphone permission prompt on macOS.
- juce: `new()` returns `Error::Backend` while another `AudioHost` exists, instead of
  panicking (JUCE can only run once per process).
- Fixed panics in `api()`, `inputs()`, `outputs()`, `input()`, `output()` and `Debug`, in
  cpal's `start()` with a huge `num_frames` (rtaudio silently truncated it to 32 bits), and in
  juce callbacks with more frames than the device buffer size.
- cpal: duplex input buffering is reworked. Input and output blocks always have the same
  length, input channels can no longer shift against each other, the buffer is sized from
  the buffer sizes the streams actually use, and it returns to its latency target after an
  xrun or clock drift instead of dropping out on every bit of jitter.
- cpal: when one direction of a duplex stream fails, the other one stops calling
  `process_fn`.
- cpal: underruns no longer print a line per sample from the audio thread; they are counted
  as xruns.
- cpal: `stop()` failed on ALSA through PipeWire (`snd_pcm_pause`).
- cpal: `num_frames` outside the device's supported buffer sizes is clamped to the supported
  range instead of making `start()` fail.
- cpal: only f32 configs are considered, as the streams always use f32. Devices without them
  fail with a clear error instead of a generic one.
- cpal: `DeviceInfo::num_channels` reported the default config's channel count instead of the
  maximum, and devices whose default config couldn't be read were missing from the lists.
- cpal, rtaudio: a `start()` that fails no longer leaves a stream capturing or a device open.
- rtaudio: a selected device that can't be found no longer silently falls back to the default
  device or to another device with the same name.
- rtaudio: `set_api` no longer switches the API when the new one can't be opened.
- `start()` with an invalid config no longer stops the running stream.
- `Debug` for `AudioHost` shows the current API, input and output instead of listing every
  device (slow, and it touched the devices).

### Documentation

- New `WINDOWS.md` explains how to build and use ASIO with the `cpal-asio` feature.
  `LINUX.md` lists the packages the juce backend and the `cpal-jack` and `cpal-pipewire`
  features need.
- Documented strict configs, monitoring the stream with `status()`, how device names are
  matched, and that `AudioHost` is only `Send` on the cpal backend.

### Internal

- Replaced the hardware test with unit tests and manual hardware tests (`#[ignore]`d, run them
  with `cargo test --test hardware -- --ignored`). CI runs `cargo test` for every backend.
- The published package only contains the sources, examples, tests, docs and licenses.

## [0.1.1] - 2026-10-19

### Changed

- Updated cpal from 0.17 to 0.18 (for the cpal backend).
- Updated rtaudio from 0.8.0 to 0.8.1 (for the rtaudio backend).
- Declared the minimum supported Rust version (`rust-version = "1.88"`) in `Cargo.toml`.

### Fixed

- The rtaudio backend no longer prints debug logs when a device is created (fixed by the
  rtaudio 0.8.1 update).
- The rtaudio backend now compiles on Linux arm64 (aarch64) (fixed by the rtaudio 0.8.1
  update).
- The compile error shown when no backend is selected no longer lists `web`, which is not a
  supported backend.

### Documentation

- Added `LINUX.md` listing the system packages the rtaudio and juce backends need on
  Ubuntu/Debian and Fedora.

### Internal

- The macOS deployment target is now set to 11.0 for builds inside this repository (via
  `.cargo/config.toml`).
- CI now runs clippy and builds every backend on Linux, macOS and Windows, and checks MSRV for
  each backend with and without serde.
- Added a daily compatibility job that builds against the latest dependency versions.

[Unreleased]: https://github.com/neodsp/audio-host/compare/0.2.0...HEAD
[0.2.0]: https://github.com/neodsp/audio-host/compare/0.1.1...0.2.0
[0.1.1]: https://github.com/neodsp/audio-host/releases/tag/0.1.1
