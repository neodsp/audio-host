use std::fmt::Debug;

use audio_blocks::{
    AudioBlockInterleaved, AudioBlockInterleavedViewMut, AudioBlockMut, AudioBlockOps,
};
use cpal::{
    SampleRate, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::RingBuffer;

pub type AudioDeviceResult<T> = Result<T, Box<dyn std::error::Error>>;

pub type Block<'a> = AudioBlockInterleavedViewMut<'a, f32>;

#[derive(Debug, Default)]
pub struct Config {
    pub num_channels: u16,
    pub sample_rate: u32,
    pub num_frames: usize,
}

#[derive(thiserror::Error, Debug)]
pub enum AudioDeviceError {
    #[error("Wanted setting not available, leaving at default")]
    NotAvailable,
}

#[derive(Debug, Clone)]
pub struct Input {
    pub name: String,
    pub num_channels: u16,
}

impl AsRef<str> for Input {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub name: String,
    pub num_channels: u16,
}

impl AsRef<str> for Output {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

pub struct AudioDevice {
    host: cpal::Host,
    host_id: cpal::HostId,
    input_device: Option<cpal::Device>,
    output_device: Option<cpal::Device>,
    output_stream: Option<Stream>,
    input_stream: Option<Stream>,
}

impl Debug for AudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDevice")
            .field("backend", &"CPAL")
            .field("is_running", &self.output_stream.is_some())
            .field("apis", &self.apis())
            .field("inputs", &self.inputs())
            .field("outputs", &self.outputs())
            .finish()
    }
}

impl AudioDevice {
    pub fn new() -> AudioDeviceResult<Self> {
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
        })
    }

    pub fn api(&self) -> String {
        self.host_id.name().to_string()
    }

    pub fn apis(&self) -> Vec<String> {
        cpal::available_hosts()
            .iter()
            .map(|api| api.name().to_string())
            .collect()
    }

    pub fn input(&self) -> Input {
        Input {
            name: self
                .input_device
                .as_ref()
                .and_then(|d| d.name().ok())
                .unwrap_or_default(),
            num_channels: self
                .input_device
                .as_ref()
                .and_then(|d| d.default_input_config().ok().map(|c| c.channels()))
                .unwrap_or_default(),
        }
    }

    pub fn output(&self) -> Output {
        Output {
            name: self
                .output_device
                .as_ref()
                .and_then(|d| d.name().ok())
                .unwrap_or_default(),
            num_channels: self
                .output_device
                .as_ref()
                .and_then(|d| d.default_output_config().ok().map(|c| c.channels()))
                .unwrap_or_default(),
        }
    }

    pub fn inputs(&self) -> Vec<Input> {
        self.host
            .input_devices()
            .ok()
            .map(|devices| {
                devices
                    .filter_map(|device| {
                        let name = device.name().ok()?;
                        let num_channels = device.default_input_config().ok()?.channels() as u16;
                        Some(Input { name, num_channels })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn outputs(&self) -> Vec<Output> {
        self.host
            .output_devices()
            .ok()
            .map(|devices| {
                devices
                    .filter_map(|device| {
                        let name = device.name().ok()?;
                        let num_channels = device.default_output_config().ok()?.channels() as u16;
                        Some(Output { name, num_channels })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_api(&mut self, name: &str) -> AudioDeviceResult<()> {
        let host_id = cpal::available_hosts()
            .iter()
            .find(|api| api.name().contains(name))
            .ok_or(AudioDeviceError::NotAvailable)?
            .clone();

        self.host = cpal::host_from_id(host_id.clone())?;
        self.host_id = host_id;

        // Update default devices for new host
        self.input_device = self.host.default_input_device();
        self.output_device = self.host.default_output_device();

        Ok(())
    }

    pub fn set_input(&mut self, input: &str) -> AudioDeviceResult<()> {
        let device = self
            .host
            .input_devices()?
            .find(|device| {
                device
                    .name()
                    .ok()
                    .map(|name| name.contains(input))
                    .unwrap_or(false)
            })
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.input_device = Some(device);
        Ok(())
    }

    pub fn set_output(&mut self, output: &str) -> AudioDeviceResult<()> {
        let device = self
            .host
            .output_devices()?
            .find(|device| {
                device
                    .name()
                    .ok()
                    .map(|name| name.contains(output))
                    .unwrap_or(false)
            })
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.output_device = Some(device);
        Ok(())
    }

    pub fn start(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block) + Send + 'static,
    ) -> AudioDeviceResult<()> {
        let has_input = self.input_device.is_some();
        let has_output = self.output_device.is_some();

        // this architecture needs at least an output device
        if !has_output {
            return Err(AudioDeviceError::NotAvailable.into());
        }

        // Get actual channel counts from devices
        let max_input_channels = self
            .input_device
            .as_ref()
            .and_then(|d| d.default_input_config().ok())
            .map(|c| c.channels())
            .unwrap_or(0);

        let max_output_channels = self
            .output_device
            .as_ref()
            .and_then(|d| d.default_output_config().ok())
            .map(|c| c.channels())
            .unwrap_or(0);

        // Limit channel count to capabilities
        let input_channels = config.num_channels.min(max_input_channels);
        let output_channels = config.num_channels.min(max_output_channels);

        // Enough space in process for the larget channel count
        let process_channels = input_channels.max(output_channels);

        // Only create ring buffer if we have input audio
        let (mut producer, mut consumer) = if has_input {
            let latency_ms = 100;
            let latency_samples = (latency_ms as f64 / 1000.0 * config.sample_rate as f64) as usize
                * input_channels as usize;
            let input_block_size = input_channels as usize * config.num_frames;
            let (mut producer, consumer) =
                RingBuffer::<f32>::new(latency_samples + 10 * input_block_size);

            // Pre-fill with silence for latency compensation
            for _ in 0..latency_samples {
                let _ = producer.push(0.0);
            }
            (Some(producer), Some(consumer))
        } else {
            (None, None)
        };

        // Start input stream if input device is selected
        if let Some(input_device) = &self.input_device {
            // Use actual device channel count for the stream
            let input_stream_config = StreamConfig {
                channels: input_channels,
                sample_rate: SampleRate(config.sample_rate),
                buffer_size: cpal::BufferSize::Fixed(config.num_frames as u32),
            };
            let input_stream = input_device.build_input_stream(
                &input_stream_config,
                move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                    if let Some(ref mut producer) = producer {
                        // Send raw input data (actual device channels)
                        for sample in data {
                            if producer.push(*sample).is_err() {
                                eprintln!(
                                    "AudioDevice: Could not push complete input into producer..."
                                );
                            }
                        }
                    }
                },
                move |err| eprintln!("Error in input stream: {:?}", err),
                None,
            )?;
            input_stream.play()?;
            self.input_stream = Some(input_stream);
        }

        // Start output stream if output device is selected
        if let Some(output_device) = &self.output_device {
            // Use actual device channel count for the stream
            let output_stream_config = StreamConfig {
                channels: output_channels,
                sample_rate: SampleRate(config.sample_rate),
                buffer_size: cpal::BufferSize::Fixed(config.num_frames as u32),
            };

            let mut process_block =
                AudioBlockInterleaved::<f32>::new(process_channels, config.num_frames);

            let output_stream = output_device.build_output_stream(
                &output_stream_config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    let num_frames = data.len() / output_channels as usize;

                    // Read input data from ring buffer if input is configured
                    if let Some(ref mut consumer) = consumer {
                        process_block.set_active_num_channels(input_channels);
                        for frame in process_block.frames_mut() {
                            for sample in frame {
                                *sample = consumer.pop().unwrap_or_else(|_| {
                                    eprintln!("AudioDevice: Could not pop sample from consumer");
                                    0.0
                                });
                            }
                        }
                    }

                    // change num_channels back to the process channels
                    process_block.set_active_num_channels(process_channels);

                    // Call user's process function
                    process_fn(process_block.view_mut());

                    // Copy from process buffer to output, handling channel mismatch
                    let mut output_view =
                        AudioBlockInterleavedViewMut::from_slice(data, output_channels, num_frames);
                    output_view.copy_from_block_resize(&process_block);
                },
                move |err| eprintln!("Error in output stream: {:?}", err),
                None,
            )?;

            output_stream.play()?;
            self.output_stream = Some(output_stream);
        }

        Ok(())
    }

    pub fn stop(&mut self) -> AudioDeviceResult<()> {
        if let Some(stream) = self.output_stream.take() {
            stream.pause()?;
        }
        if let Some(stream) = self.input_stream.take() {
            stream.pause()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use audio_blocks::AudioBlock;

    use super::*;

    #[test]
    fn cpal_test() {
        let mut device = AudioDevice::new().unwrap();
        dbg!(device.apis());
        dbg!(device.inputs());
        dbg!(device.outputs());

        dbg!(device.api());
        dbg!(device.input());
        dbg!(device.output());

        device.set_api(&device.api()).unwrap();
        device.set_input(&device.input().name).unwrap();
        device.set_output(&device.output().name).unwrap();

        device
            .start(
                Config {
                    sample_rate: 48000,
                    num_frames: 512,
                    num_channels: 2,
                },
                |block| {
                    assert_eq!(block.num_frames(), 512);
                },
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(10));

        device.stop().unwrap();
    }
}
