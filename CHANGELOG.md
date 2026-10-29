# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- `Error` is now `#[non_exhaustive]`.
- `AudioBackend` has a new required method, `status()`.
- `set_api`, `set_input` and `set_output` now prefer an exact name match over the first name
  containing the text.
- juce: `input()` and `output()` are empty until `start()` if no device was selected, because
  JUCE only knows its default devices after opening them.
- juce: `set_api` returns `Error::NotFound` for unknown APIs and APIs without devices (it
  used to return `Ok`).
- cpal, rtaudio: `set_api` now stops a running stream, like the juce backend.

### Added

- `status()` reports whether the stream is still running, why it stopped on its own
  (`StreamError::DeviceLost` or `StreamError::Backend`) and how many xruns happened.
- The cpal backend now supports input-only streams.

### Fixed

- The unused block in input-only and output-only streams now has 0 channels and 0 frames on
  every backend (cpal and rtaudio passed 1 channel, juce passed the callback's frame count).
- juce: callbacks with more frames than the device buffer size are split instead of
  panicking on the audio thread.
- cpal: when one direction of a duplex stream fails, the other one stops calling
  `process_fn` and counting xruns.
- juce: the devices selected with `set_input` / `set_output` were ignored; JUCE always opened
  its default devices.
- juce: only the devices a stream uses are opened. Output-only streams no longer open the
  input device (which triggered the microphone permission prompt on macOS).
- juce: `new()` no longer opens devices, so it no longer fails when a default device is busy.
- juce: `stop()` now closes the device instead of keeping it open.
- cpal: `stop()` failed on ALSA through PipeWire (`snd_pcm_pause`).
- cpal: duplex streams could pass input and output blocks of different lengths, and the
  input drifted, when the driver delivered a different block size than `num_frames`.
- cpal: in duplex streams with more than one input channel, an overrun or underrun of the
  input ring buffer could shift the input channels against each other until the next one.
  Only whole frames are buffered now.
- cpal: underruns no longer print a line per sample from the audio thread; they are counted
  as xruns.
- cpal: `DeviceInfo::num_channels` reported the default config's channel count instead of the
  maximum, and devices whose default config couldn't be read were missing from the lists.
- rtaudio: a selected device that can't be found no longer silently falls back to the default
  device or to another device with the same name.
- rtaudio: `set_api` no longer switches the API when the new one can't be opened.
- cpal: a duplex `start()` whose output fails no longer leaves the input stream capturing.
- cpal: `num_frames` outside the device's supported buffer sizes no longer makes `start()`
  fail; it is clamped to the supported range.
- cpal: only f32 configs are considered, as the streams always use f32. Devices without them
  fail with a clear error instead of a generic one.
- `api()`, `inputs()`, `outputs()`, `input()`, `output()` and `Debug` could panic.

### Documentation

- `LINUX.md` lists the JACK development package the juce backend needs.
- Documented strict configs, monitoring the stream with `status()` and how device names are
  matched.
- Doc examples that open audio devices no longer run during `cargo test`.
- rtaudio: documented that `status()` relies on rtaudio's process-wide error callback.

### Internal

- Replaced the hardware test with unit tests and manual hardware tests (`#[ignore]`d, run them
  with `cargo test --test hardware -- --ignored`).
- CI now runs `cargo test` for every backend.

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

[Unreleased]: https://github.com/neodsp/audio-host/compare/0.1.1...HEAD
[0.1.1]: https://github.com/neodsp/audio-host/releases/tag/0.1.1
