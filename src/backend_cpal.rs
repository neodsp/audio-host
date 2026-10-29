use std::{fmt::Debug, sync::Arc};

use audio_blocks::{AudioBlockMut, Interleaved};
use cpal::{
    Stream, StreamConfig, SupportedStreamConfigRange,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::RingBuffer;

use crate::{
    AudioBackend, Block, BlockMut, Config, DeviceInfo, Error, StatusTracker, StreamError,
    StreamStatus, find_by_name,
};

/// Input latency added to duplex streams. cpal runs input and output as two streams, so the
/// input goes through a ring buffer that absorbs timing differences between them.
const DUPLEX_LATENCY_MS: u32 = 100;

/// Audio host using cpal.
///
/// Duplex streams (input and output) add 100 ms (`DUPLEX_LATENCY_MS`) of input latency.
pub struct AudioHost {
    host: cpal::Host,
    host_id: cpal::HostId,
    input_device: Option<cpal::Device>,
    output_device: Option<cpal::Device>,
    output_stream: Option<Stream>,
    input_stream: Option<Stream>,
    status: Arc<StatusTracker>,
}

impl Debug for AudioHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioHost")
            .field("backend", &"CPAL")
            .field(
                "is_running",
                &(self.output_stream.is_some() || self.input_stream.is_some()),
            )
            .field("apis", &self.apis())
            .field("inputs", &self.inputs())
            .field("outputs", &self.outputs())
            .finish()
    }
}

impl AudioBackend for AudioHost {
    fn new() -> Result<Self, Error> {
        let host = cpal::default_host();
        let host_id = host.id();

        let input_device = host.default_input_device();
        let output_device = host.default_output_device();

        Ok(Self {
            host,
            host_id,
            input_device,
            output_device,
            output_stream: None,
            input_stream: None,
            status: Arc::default(),
        })
    }

    fn api(&self) -> String {
        self.host_id.name().to_string()
    }

    fn apis(&self) -> Vec<String> {
        cpal::available_hosts()
            .iter()
            .map(|api| api.name().to_string())
            .collect()
    }

    fn input(&self) -> String {
        self.input_device
            .as_ref()
            .and_then(device_name)
            .unwrap_or_default()
    }

    fn output(&self) -> String {
        self.output_device
            .as_ref()
            .and_then(device_name)
            .unwrap_or_default()
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        device_infos(self.host.input_devices().ok(), |device| {
            device.supported_input_configs().ok()
        })
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        device_infos(self.host.output_devices().ok(), |device| {
            device.supported_output_configs().ok()
        })
    }

    fn set_api(&mut self, name: &str) -> Result<(), Error> {
        let host_id = find_by_name(
            cpal::available_hosts(),
            |api| Some(api.name().to_string()),
            name,
        )
        .ok_or(Error::NotFound)?;

        // nothing changes unless the new host can be opened
        let host = cpal::host_from_id(host_id).map_err(|e| Error::Backend(Box::new(e)))?;

        // a running stream belongs to the old host, so stop it (like the other backends)
        self.stop()?;
        self.host = host;
        self.host_id = host_id;

        self.input_device = self.host.default_input_device();
        self.output_device = self.host.default_output_device();

        Ok(())
    }

    fn set_input(&mut self, input: &str) -> Result<(), Error> {
        let devices = self
            .host
            .input_devices()
            .map_err(|e| Error::Backend(Box::new(e)))?;
        let device = find_by_name(devices, device_name, input).ok_or(Error::NotFound)?;

        self.input_device = Some(device);
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> Result<(), Error> {
        let devices = self
            .host
            .output_devices()
            .map_err(|e| Error::Backend(Box::new(e)))?;
        let device = find_by_name(devices, device_name, output).ok_or(Error::NotFound)?;

        self.output_device = Some(device);
        Ok(())
    }

    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error> {
        self.stop()?;
        config.validate()?;
        self.status.reset();

        // running before the streams start, so errors they report right away aren't ignored.
        // start then still succeeds, and status() reports the error.
        self.status.set_running(true);
        let result = self.open_streams(config, process_fn);
        if result.is_err() {
            self.status.reset();
        }
        result
    }

    fn stop(&mut self) -> Result<(), Error> {
        // not running first, so errors while closing aren't reported
        self.status.set_running(false);
        // dropping a stream stops and closes it; pausing first fails on devices that can't
        // pause (e.g. ALSA through PipeWire)
        self.output_stream = None;
        self.input_stream = None;
        Ok(())
    }

    fn status(&self) -> StreamStatus {
        self.status.status()
    }
}

impl AudioHost {
    /// Checks the request and opens and starts the streams. Stores them only if all of them
    /// started.
    fn open_streams(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error> {
        let num_input_channels = config.num_input_channels;
        let num_output_channels = config.num_output_channels;
        let has_input = num_input_channels > 0;
        let has_output = num_output_channels > 0;

        // check the request up front, so a running stream always matches the config
        if has_input && self.input_device.is_none() {
            return Err(Error::UnsupportedInputChannels {
                requested: num_input_channels,
                supported: 0,
            });
        }
        if has_output && self.output_device.is_none() {
            return Err(Error::UnsupportedOutputChannels {
                requested: num_output_channels,
                supported: 0,
            });
        }

        // devices that can't list their configs are left to cpal to report
        let input_ranges = match (&self.input_device, has_input) {
            (Some(device), true) => f32_ranges(
                device
                    .supported_input_configs()
                    .map(|c| c.collect())
                    .unwrap_or_default(),
            )?,
            _ => Vec::new(),
        };
        let output_ranges = match (&self.output_device, has_output) {
            (Some(device), true) => f32_ranges(
                device
                    .supported_output_configs()
                    .map(|c| c.collect())
                    .unwrap_or_default(),
            )?,
            _ => Vec::new(),
        };

        let rate_supported = |ranges: &[SupportedStreamConfigRange]| {
            ranges.is_empty() || supports_rate(ranges, config.sample_rate)
        };
        if !rate_supported(&input_ranges) || !rate_supported(&output_ranges) {
            return Err(Error::UnsupportedSampleRate {
                requested: config.sample_rate,
                available: available_rates(
                    &input_ranges,
                    num_input_channels,
                    &output_ranges,
                    num_output_channels,
                ),
            });
        }

        check_channels(&input_ranges, num_input_channels, config.sample_rate).map_err(
            |supported| Error::UnsupportedInputChannels {
                requested: num_input_channels,
                supported,
            },
        )?;
        check_channels(&output_ranges, num_output_channels, config.sample_rate).map_err(
            |supported| Error::UnsupportedOutputChannels {
                requested: num_output_channels,
                supported,
            },
        )?;

        let input_stream_config = stream_config(&input_ranges, num_input_channels, &config);

        // input-only: run the process callback directly in the input stream
        if !has_output {
            let Some(input_device) = &self.input_device else {
                unreachable!("checked above");
            };
            let input_stream = input_device
                .build_input_stream(
                    input_stream_config,
                    move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                        let input_block = Block::from_slice(data, num_input_channels);
                        // empty output block (0 channels, 0 frames), like the other backends
                        let output_block = BlockMut::from_slice_limited(&mut [], 0, 0, 0, 0);
                        process_fn(input_block, output_block);
                    },
                    error_handler(&self.status),
                    None,
                )
                .map_err(|e| Error::Backend(Box::new(e)))?;
            input_stream
                .play()
                .map_err(|e| Error::Backend(Box::new(e)))?;
            self.input_stream = Some(input_stream);
            return Ok(());
        }

        // with an output, the process callback runs in the output stream and gets the input
        // through a ring buffer
        let (mut producer, mut consumer) = if has_input {
            let latency_samples = (DUPLEX_LATENCY_MS as usize * config.sample_rate as usize / 1000)
                * num_input_channels as usize;
            let input_block_size = num_input_channels as usize * config.num_frames;
            let (mut producer, consumer) =
                RingBuffer::<f32>::new(latency_samples + 10 * input_block_size);

            for _ in 0..latency_samples {
                let _ = producer.push(0.0);
            }
            (Some(producer), Some(consumer))
        } else {
            (None, None)
        };

        // ring buffer overruns and underruns count as xruns
        let input_status = self.status.clone();
        let output_status = self.status.clone();

        // streams stay local until both are playing, so on any error they are dropped (closed)
        // instead of an input stream being left running
        let mut input_stream = None;
        if has_input && let Some(input_device) = &self.input_device {
            let stream = input_device
                .build_input_stream(
                    input_stream_config,
                    move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                        // the output stream failed: stop feeding it (and counting overruns)
                        if !input_status.is_running() {
                            return;
                        }
                        if let Some(ref mut producer) = producer
                            && !push_frames(producer, data, num_input_channels)
                        {
                            input_status.add_xrun();
                        }
                    },
                    error_handler(&self.status),
                    None,
                )
                .map_err(|e| Error::Backend(Box::new(e)))?;
            input_stream = Some(stream);
        }

        let mut output_stream = None;
        if let Some(output_device) = &self.output_device {
            let output_stream_config = stream_config(&output_ranges, num_output_channels, &config);

            let mut input_block = if has_input {
                Interleaved::new(num_input_channels, config.num_frames)
            } else {
                Interleaved::new(0, 0)
            };
            let max_chunk_samples = config.num_frames * num_output_channels as usize;

            let stream = output_device
                .build_output_stream(
                    output_stream_config,
                    move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                        // the stream stopped (e.g. the input device was lost): output silence
                        // without calling `process_fn` or counting underruns
                        if !output_status.is_running() {
                            data.fill(0.0);
                            return;
                        }
                        // the driver may deliver more or fewer frames than requested. Larger
                        // callbacks are split, so input and output blocks always have the same
                        // length and never exceed `num_frames` (no allocation needed)
                        for chunk in data.chunks_mut(max_chunk_samples) {
                            if let Some(ref mut consumer) = consumer {
                                let num_frames = chunk.len() / num_output_channels as usize;
                                input_block.set_num_frames_visible(num_frames);
                                // all input channels are visible, so the visible frames are
                                // the start of the raw data
                                let samples = num_frames * num_input_channels as usize;
                                let input = &mut input_block.raw_data_mut()[..samples];
                                if !pop_frames(consumer, input, num_input_channels) {
                                    output_status.add_xrun();
                                }
                            }

                            let output_block = BlockMut::from_slice(chunk, num_output_channels);
                            process_fn(input_block.view(), output_block);
                        }
                    },
                    error_handler(&self.status),
                    None,
                )
                .map_err(|e| Error::Backend(Box::new(e)))?;
            output_stream = Some(stream);
        }

        for stream in input_stream.iter().chain(output_stream.iter()) {
            stream.play().map_err(|e| Error::Backend(Box::new(e)))?;
        }
        self.input_stream = input_stream;
        self.output_stream = output_stream;
        Ok(())
    }
}

/// Pushes the whole frames of `data` that fit into the ring buffer and drops the rest.
/// Returns `false` if frames were dropped.
///
/// Only whole frames are moved, so the channels never get out of step: pushing sample by
/// sample could drop the first sample of a frame and then push the second one, once the
/// consumer frees a slot in between.
fn push_frames(producer: &mut rtrb::Producer<f32>, data: &[f32], num_channels: u16) -> bool {
    let num_channels = num_channels as usize;
    // free slots only grow while the consumer runs, so all of them can be pushed
    let fit = producer.slots().min(data.len()) / num_channels * num_channels;
    let (pushed, _) = producer.push_partial_slice(&data[..fit]);
    pushed.len() == data.len()
}

/// Fills `out` with whole frames from the ring buffer and the rest with silence.
/// Returns `false` if there weren't enough frames.
///
/// Only whole frames are moved, for the same reason as in [`push_frames`].
fn pop_frames(consumer: &mut rtrb::Consumer<f32>, out: &mut [f32], num_channels: u16) -> bool {
    let num_channels = num_channels as usize;
    // available samples only grow while the producer runs, so all of them can be popped
    let available = consumer.slots().min(out.len()) / num_channels * num_channels;
    let (popped, _) = consumer.pop_partial_slice(&mut out[..available]);
    let popped = popped.len();
    out[popped..].fill(0.0);
    popped == out.len()
}

/// Error callback for a cpal stream, feeding the stream status.
fn error_handler(status: &Arc<StatusTracker>) -> impl FnMut(cpal::Error) + Send + 'static {
    let status = status.clone();
    move |error| match error.kind() {
        cpal::ErrorKind::Xrun => status.add_xrun(),
        // the stream keeps running in these cases (RealtimeDenied: audio plays without
        // real-time priority)
        cpal::ErrorKind::DeviceChanged
        | cpal::ErrorKind::ResourceExhausted
        | cpal::ErrorKind::RealtimeDenied => {}
        cpal::ErrorKind::DeviceNotAvailable => status.fail(StreamError::DeviceLost),
        _ => status.fail(StreamError::Backend(error.to_string())),
    }
}

/// Lists `devices` with the most f32 channels `configs` reports for each.
fn device_infos<C: Iterator<Item = SupportedStreamConfigRange>>(
    devices: Option<impl Iterator<Item = cpal::Device>>,
    configs: impl Fn(&cpal::Device) -> Option<C>,
) -> Vec<DeviceInfo> {
    devices
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let name = device_name(&device)?;
            let num_channels = configs(&device)
                .and_then(|configs| configs.filter(is_f32).map(|c| c.channels()).max());
            Some(DeviceInfo { name, num_channels })
        })
        .collect()
}

/// The stream config for one direction of `config`.
fn stream_config(
    ranges: &[SupportedStreamConfigRange],
    channels: u16,
    config: &Config,
) -> StreamConfig {
    StreamConfig {
        channels,
        sample_rate: config.sample_rate,
        buffer_size: buffer_size(ranges, channels, config.sample_rate, config.num_frames),
    }
}

/// The device name, `None` if cpal can't read it (e.g. the device was just unplugged).
fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

/// cpal reports sample rate ranges, so errors list the common rates inside them.
const COMMON_SAMPLE_RATES: [u32; 11] = [
    8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
];

/// The ranges the streams can use: they always request f32 samples. Errors if the device lists
/// configs but none with f32. An empty list (configs unknown) is passed on.
fn f32_ranges(
    ranges: Vec<SupportedStreamConfigRange>,
) -> Result<Vec<SupportedStreamConfigRange>, Error> {
    if ranges.is_empty() {
        return Ok(ranges);
    }
    let f32_ranges: Vec<_> = ranges.into_iter().filter(is_f32).collect();
    if f32_ranges.is_empty() {
        return Err(Error::Backend(
            "the device doesn't support 32-bit float samples".into(),
        ));
    }
    Ok(f32_ranges)
}

fn is_f32(range: &SupportedStreamConfigRange) -> bool {
    range.sample_format() == cpal::SampleFormat::F32
}

/// The common sample rates both directions support with the requested channel counts, so
/// retrying with one of them works. A direction without a config for its channel count uses
/// all its configs, as retrying fails on the channel count anyway.
fn available_rates(
    input_ranges: &[SupportedStreamConfigRange],
    num_input_channels: u16,
    output_ranges: &[SupportedStreamConfigRange],
    num_output_channels: u16,
) -> Vec<u32> {
    let with_channels = |ranges: &[SupportedStreamConfigRange], channels: u16| {
        let matching: Vec<_> = ranges
            .iter()
            .filter(|r| r.channels() == channels)
            .cloned()
            .collect();
        if matching.is_empty() {
            ranges.to_vec()
        } else {
            matching
        }
    };
    let input_ranges = with_channels(input_ranges, num_input_channels);
    let output_ranges = with_channels(output_ranges, num_output_channels);
    COMMON_SAMPLE_RATES
        .into_iter()
        .filter(|&rate| {
            [&input_ranges, &output_ranges]
                .iter()
                .all(|r| r.is_empty() || supports_rate(r, rate))
        })
        .collect()
}

/// The buffer size to request: `num_frames`, clamped to the range the device supports for this
/// config. A fixed size outside it makes cpal fail, while `num_frames` is only a request.
fn buffer_size(
    ranges: &[SupportedStreamConfigRange],
    channels: u16,
    sample_rate: u32,
    num_frames: usize,
) -> cpal::BufferSize {
    let num_frames = u32::try_from(num_frames).unwrap_or(u32::MAX);
    let supported = ranges
        .iter()
        .filter(|r| r.channels() == channels)
        .filter(|r| r.min_sample_rate() <= sample_rate && sample_rate <= r.max_sample_rate())
        .find_map(|r| match *r.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if min <= max => Some((min, max)),
            _ => None,
        });
    match supported {
        Some((min, max)) => cpal::BufferSize::Fixed(num_frames.clamp(min, max)),
        // unknown range: request it and let cpal decide
        None => cpal::BufferSize::Fixed(num_frames),
    }
}

fn supports_rate(ranges: &[SupportedStreamConfigRange], sample_rate: u32) -> bool {
    ranges
        .iter()
        .any(|r| r.min_sample_rate() <= sample_rate && sample_rate <= r.max_sample_rate())
}

/// Checks that the device supports exactly `requested` channels at `sample_rate`.
/// On failure returns the closest supported count: the largest one below `requested`, or the
/// smallest one if all are larger. Passes if the device reports nothing.
fn check_channels(
    ranges: &[SupportedStreamConfigRange],
    requested: u16,
    sample_rate: u32,
) -> Result<(), u16> {
    let counts: Vec<u16> = ranges
        .iter()
        .filter(|r| r.min_sample_rate() <= sample_rate && sample_rate <= r.max_sample_rate())
        .map(|r| r.channels())
        .collect();
    if counts.is_empty() || counts.contains(&requested) {
        return Ok(());
    }
    let closest = counts
        .iter()
        .copied()
        .filter(|&c| c < requested)
        .max()
        .or_else(|| counts.iter().copied().min());
    Err(closest.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use cpal::{SampleFormat, SupportedBufferSize};

    use super::*;

    fn range(channels: u16, min_rate: u32, max_rate: u32) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            min_rate,
            max_rate,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        )
    }

    fn i16_range(channels: u16, min_rate: u32, max_rate: u32) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            min_rate,
            max_rate,
            SupportedBufferSize::Unknown,
            SampleFormat::I16,
        )
    }

    #[test]
    fn only_f32_ranges_are_used() {
        let ranges = f32_ranges(vec![i16_range(8, 44100, 96000), range(2, 44100, 48000)]).unwrap();
        assert_eq!(ranges.len(), 1);
        // 8 channels are only available as i16, which the streams can't open
        assert_eq!(check_channels(&ranges, 8, 48000), Err(2));
    }

    #[test]
    fn devices_without_f32_are_rejected() {
        let result = f32_ranges(vec![i16_range(2, 44100, 48000)]);
        assert!(matches!(result, Err(Error::Backend(_))));
        // unknown configs are still left to cpal
        assert!(f32_ranges(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn suggested_rates_match_the_channel_count() {
        // 96 kHz only works in mono, so a stereo request must not be offered it
        let input = [range(1, 44100, 96000), range(2, 44100, 48000)];
        assert_eq!(available_rates(&input, 2, &[], 0), vec![44100, 48000]);
        assert_eq!(
            available_rates(&input, 1, &[], 0),
            vec![44100, 48000, 88200, 96000]
        );
        // no config for the channel count: fall back to all configs
        assert_eq!(
            available_rates(&input, 4, &[], 0),
            vec![44100, 48000, 88200, 96000]
        );
    }

    #[test]
    fn suggested_rates_work_for_both_directions() {
        let input = [range(2, 44100, 96000)];
        let output = [range(2, 8000, 48000)];
        assert_eq!(available_rates(&input, 2, &output, 2), vec![44100, 48000]);
    }

    fn range_with_buffer(channels: u16, min: u32, max: u32) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            44100,
            48000,
            SupportedBufferSize::Range { min, max },
            SampleFormat::F32,
        )
    }

    #[test]
    fn buffer_size_is_clamped_to_the_supported_range() {
        let ranges = [
            range_with_buffer(2, 64, 480),
            range_with_buffer(4, 1024, 4096),
        ];
        let size = |channels, frames| buffer_size(&ranges, channels, 48000, frames);
        assert_eq!(size(2, 256), cpal::BufferSize::Fixed(256));
        assert_eq!(size(2, 512), cpal::BufferSize::Fixed(480));
        assert_eq!(size(2, 16), cpal::BufferSize::Fixed(64));
        // the range of the matching channel count is used
        assert_eq!(size(4, 512), cpal::BufferSize::Fixed(1024));
        // unknown ranges keep the request
        assert_eq!(
            buffer_size(&[range(2, 44100, 48000)], 2, 48000, 512),
            cpal::BufferSize::Fixed(512)
        );
    }

    #[test]
    fn exact_channel_count_passes() {
        let ranges = [range(2, 44100, 48000), range(4, 44100, 48000)];
        assert_eq!(check_channels(&ranges, 2, 48000), Ok(()));
        assert_eq!(check_channels(&ranges, 4, 44100), Ok(()));
    }

    #[test]
    fn unknown_device_configs_pass() {
        assert_eq!(check_channels(&[], 8, 48000), Ok(()));
    }

    #[test]
    fn suggests_largest_count_below_request() {
        let ranges = [range(2, 48000, 48000), range(4, 48000, 48000)];
        assert_eq!(check_channels(&ranges, 3, 48000), Err(2));
        assert_eq!(check_channels(&ranges, 200, 48000), Err(4));
    }

    #[test]
    fn suggests_smallest_count_if_all_are_larger() {
        let ranges = [range(2, 48000, 48000), range(4, 48000, 48000)];
        assert_eq!(check_channels(&ranges, 1, 48000), Err(2));
    }

    #[test]
    fn ignores_configs_at_other_sample_rates() {
        let ranges = [range(2, 48000, 48000), range(8, 96000, 96000)];
        assert_eq!(check_channels(&ranges, 8, 48000), Err(2));
        assert_eq!(check_channels(&ranges, 8, 96000), Ok(()));
    }

    #[test]
    fn sample_rate_must_be_inside_a_range() {
        let ranges = [range(2, 44100, 48000)];
        assert!(supports_rate(&ranges, 44100));
        assert!(supports_rate(&ranges, 48000));
        assert!(!supports_rate(&ranges, 96000));
        assert!(!supports_rate(&[], 48000));
    }

    #[test]
    fn ring_buffer_drops_and_pads_whole_frames() {
        // room for 2.5 stereo frames
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(5);
        assert!(!push_frames(
            &mut producer,
            &[1.0, 2.0, 1.0, 2.0, 1.0, 2.0],
            2
        ));
        // the half frame that would fit is dropped as well
        assert_eq!(producer.slots(), 1);

        let mut out = [9.0; 6];
        assert!(!pop_frames(&mut consumer, &mut out, 2));
        assert_eq!(out, [1.0, 2.0, 1.0, 2.0, 0.0, 0.0]);

        assert!(push_frames(&mut producer, &[3.0, 4.0], 2));
        let mut out = [9.0; 2];
        assert!(pop_frames(&mut consumer, &mut out, 2));
        assert_eq!(out, [3.0, 4.0]);
    }

    #[test]
    fn ring_buffer_keeps_channels_in_step_under_xruns() {
        // a tiny buffer, so both threads keep hitting overruns and underruns
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(7);
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                for i in 1..200_000 {
                    // each frame is (x, -x), so a shift between channels is easy to spot
                    let x = (i % 1000 + 1) as f32;
                    push_frames(&mut producer, &[x, -x, x + 0.5, -x - 0.5], 2);
                }
                done.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let mut out = [0.0; 6];
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                pop_frames(&mut consumer, &mut out, 2);
                for frame in out.chunks(2) {
                    assert_eq!(frame[0], -frame[1], "channels out of step: {out:?}");
                }
            }
        });
    }

    /// Feeds `kind` to the error handler of a running stream and returns the status after.
    fn status_after(kind: cpal::ErrorKind) -> StreamStatus {
        let status = Arc::new(StatusTracker::default());
        status.set_running(true);
        error_handler(&status)(cpal::Error::new(kind));
        status.status()
    }

    #[test]
    fn error_handler_keeps_running_on_non_fatal_errors() {
        for kind in [
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::ResourceExhausted,
            cpal::ErrorKind::RealtimeDenied,
        ] {
            let status = status_after(kind);
            assert!(status.running, "{kind:?}");
            assert_eq!(status.error, None, "{kind:?}");
        }
    }

    #[test]
    fn error_handler_counts_xruns() {
        let status = status_after(cpal::ErrorKind::Xrun);
        assert!(status.running);
        assert_eq!(status.xruns, 1);
    }

    #[test]
    fn error_handler_stops_on_fatal_errors() {
        let status = status_after(cpal::ErrorKind::DeviceNotAvailable);
        assert!(!status.running);
        assert_eq!(status.error, Some(StreamError::DeviceLost));

        let status = status_after(cpal::ErrorKind::StreamInvalidated);
        assert!(!status.running);
        assert!(matches!(status.error, Some(StreamError::Backend(_))));
    }
}
