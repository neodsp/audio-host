use std::fmt::Debug;

use audio_blocks::{
    AudioBlockInterleaved, AudioBlockInterleavedView, AudioBlockInterleavedViewMut,
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
    name: String,
    num_channels: u16,
}

#[derive(Debug, Clone)]
pub struct Output {
    name: String,
    num_channels: u16,
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

    pub fn set_input(&mut self, input: &Input) -> AudioDeviceResult<()> {
        let device = self
            .host
            .input_devices()?
            .find(|device| {
                device
                    .name()
                    .ok()
                    .map(|name| name.contains(&input.name))
                    .unwrap_or(false)
            })
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.input_device = Some(device);
        Ok(())
    }

    pub fn set_output(&mut self, output: &Output) -> AudioDeviceResult<()> {
        let device = self
            .host
            .output_devices()?
            .find(|device| {
                device
                    .name()
                    .ok()
                    .map(|name| name.contains(&output.name))
                    .unwrap_or(false)
            })
            .ok_or(AudioDeviceError::NotAvailable)?;

        self.output_device = Some(device);
        Ok(())
    }

    pub fn start(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block<'_>) + Send + 'static,
    ) -> AudioDeviceResult<()> {
        let output_device = self
            .output_device
            .as_ref()
            .ok_or(AudioDeviceError::NotAvailable)?;
        let input_device = self
            .input_device
            .as_ref()
            .ok_or(AudioDeviceError::NotAvailable)?;

        let latency_ms = 10;
        let latency_samples = (latency_ms as f64 / 1000.0 * config.sample_rate as f64) as usize
            * config.num_channels as usize;
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(latency_samples * 2);

        // Pre-fill with silence for latency compensation
        for _ in 0..latency_samples {
            let _ = producer.push(0.0);
        }

        let stream_config = StreamConfig {
            channels: config.num_channels,
            sample_rate: SampleRate(config.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(config.num_frames as u32),
        };

        let input_stream = input_device.build_input_stream(
            &stream_config,
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                for sample in data {
                    if producer.push(*sample).is_err() {
                        eprintln!("AudioDevice: Could not push complete input into producer...");
                    }
                }
            },
            move |err| eprintln!("Error in input stream: {:?}", err),
            None,
        )?;

        let mut input_block =
            AudioBlockInterleaved::<f32>::new(config.num_channels, config.num_frames);

        let output_stream = output_device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                // Read input data from ring buffer
                for sample in input_block.raw_data_mut() {
                    *sample = if let Ok(s) = consumer.pop() {
                        s
                    } else {
                        eprintln!("AudioDevice: Could not pull new samples from consumer...");
                        0.0
                    };
                }

                let mut output_view = AudioBlockInterleavedViewMut::from_slice(
                    data,
                    config.num_channels,
                    data.len() / config.num_channels as usize,
                );

                // Copy input to output (passthrough)
                for (i, o) in input_block.frames().zip(output_view.frames_mut()) {
                    let copy_len = i.len().min(o.len());
                    o[..copy_len].copy_from_slice(&i[..copy_len]);
                }

                // Call user's process function
                process_fn(output_view);
            },
            move |err| eprintln!("Error in output stream: {:?}", err),
            None,
        )?;

        input_stream.play()?;
        output_stream.play()?;

        self.input_stream = Some(input_stream);
        self.output_stream = Some(output_stream);

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
        device.set_input(&device.input()).unwrap();
        device.set_output(&device.output()).unwrap();

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
