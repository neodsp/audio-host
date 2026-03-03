use std::fmt::Debug;

use audio_blocks::Interleaved;
use cpal::{
    Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::RingBuffer;

use crate::{
    AudioDeviceError, AudioDeviceResult, AudioDeviceTrait, Block, BlockMut, Config, DeviceInfo,
};

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

impl AudioDeviceTrait for AudioDevice {
    fn new() -> AudioDeviceResult<Self> {
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
            .and_then(|d| Some(d.description().unwrap().name().to_string()))
            .unwrap_or_default()
    }

    fn output(&self) -> String {
        self.output_device
            .as_ref()
            .and_then(|d| Some(d.description().unwrap().name().to_string()))
            .unwrap_or_default()
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        self.host
            .input_devices()
            .ok()
            .map(|devices| {
                devices
                    .filter_map(|device| {
                        let name = device.description().unwrap().name().to_string();
                        let num_channels = device.default_input_config().ok()?.channels() as u16;
                        Some(DeviceInfo { name, num_channels })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        self.host
            .output_devices()
            .ok()
            .map(|devices| {
                devices
                    .filter_map(|device| {
                        let name = device.description().unwrap().name().to_string();
                        let num_channels = device.default_output_config().ok()?.channels() as u16;
                        Some(DeviceInfo { name, num_channels })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn set_api(&mut self, name: &str) -> AudioDeviceResult<()> {
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

    fn set_input(&mut self, input: &str) -> AudioDeviceResult<()> {
        let device = self
            .host
            .input_devices()?
            .find(|device| device.description().unwrap().name().contains(input))
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.input_device = Some(device);
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> AudioDeviceResult<()> {
        let device = self
            .host
            .output_devices()?
            .find(|device| device.description().unwrap().name().contains(output))
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.output_device = Some(device);
        Ok(())
    }

    fn start(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> AudioDeviceResult<()> {
        let has_input = self.input_device.is_some() && config.num_input_channels > 0;
        let has_output = self.output_device.is_some() && config.num_output_channels > 0;

        // this architecture needs at least an output device
        if !has_output {
            return Err(AudioDeviceError::NotAvailable.into());
        }

        // Only create ring buffer if we have input audio
        let (mut producer, mut consumer) = if has_input {
            let latency_ms = 100;
            let latency_samples = (latency_ms as f64 / 1000.0 * config.sample_rate as f64) as usize
                * config.num_input_channels as usize;
            let input_block_size = config.num_input_channels as usize * config.num_frames;
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

        // Start input stream if input device is selected and channels > 0
        if has_input {
            if let Some(input_device) = &self.input_device {
                let input_stream_config = StreamConfig {
                    channels: config.num_input_channels,
                    sample_rate: config.sample_rate,
                    buffer_size: cpal::BufferSize::Fixed(config.num_frames as u32),
                };
                let input_stream = input_device.build_input_stream(
                    &input_stream_config,
                    move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                        if let Some(ref mut producer) = producer {
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
        }

        // Start output stream if output device is selected
        if let Some(output_device) = &self.output_device {
            // Use actual device channel count for the stream
            let output_stream_config = StreamConfig {
                channels: config.num_output_channels,
                sample_rate: config.sample_rate,
                buffer_size: cpal::BufferSize::Fixed(config.num_frames as u32),
            };

            let mut input_block = if has_input {
                Interleaved::new(config.num_input_channels, config.num_frames)
            } else {
                Interleaved::new(1, 0)
            };

            let output_stream = output_device.build_output_stream(
                &output_stream_config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    // Read input data from ring buffer if input is configured
                    if let Some(ref mut consumer) = consumer {
                        for frame in input_block.frames_mut() {
                            for sample in frame {
                                *sample = consumer.pop().unwrap_or_else(|_| {
                                    eprintln!("AudioDevice: Could not pop sample from consumer");
                                    0.0
                                });
                            }
                        }
                    }

                    let output_block = BlockMut::from_slice(data, config.num_output_channels);

                    // Call user's process function
                    process_fn(input_block.view(), output_block);
                },
                move |err| eprintln!("Error in output stream: {:?}", err),
                None,
            )?;

            output_stream.play()?;
            self.output_stream = Some(output_stream);
        }

        Ok(())
    }

    fn stop(&mut self) -> AudioDeviceResult<()> {
        if let Some(stream) = self.output_stream.take() {
            stream.pause()?;
        }
        if let Some(stream) = self.input_stream.take() {
            stream.pause()?;
        }
        Ok(())
    }
}
