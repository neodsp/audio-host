use std::fmt::Debug;

use rtaudio::{DeviceParams, Host, StreamConfig, StreamFlags, StreamHandle};

use crate::{
    AudioDeviceError, AudioDeviceResult, AudioDeviceTrait, Block, BlockMut, Config, DeviceInfo,
};

pub struct AudioDevice {
    api: rtaudio::Api,
    input_device: rtaudio::DeviceInfo,
    output_device: rtaudio::DeviceInfo,
    stream_handle: Option<StreamHandle>,
}

impl Debug for AudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDevice")
            .field("backend", &"RtAudio")
            .field("is_running", &self.stream_handle.is_some())
            .field("apis", &self.apis())
            .field("inputs", &self.inputs())
            .field("outputs", &self.outputs())
            .finish()
    }
}

impl AudioDeviceTrait for AudioDevice {
    fn new() -> AudioDeviceResult<Self> {
        let host = Host::default();
        let input_device = host
            .iter_input_devices()
            .find(|d| d.is_default_input)
            .cloned()
            .unwrap();
        let output_device = host
            .iter_output_devices()
            .find(|d| d.is_default_output)
            .cloned()
            .unwrap();
        Ok(Self {
            api: host.api(),
            input_device,
            output_device,
            stream_handle: None,
        })
    }

    fn api(&self) -> String {
        self.api.get_display_name()
    }

    fn apis(&self) -> Vec<String> {
        rtaudio::compiled_apis()
            .iter()
            .map(|a| a.get_display_name())
            .collect()
    }

    fn input(&self) -> String {
        self.input_device.name().to_string()
    }

    fn output(&self) -> String {
        self.output_device.name().to_string()
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        Host::new(self.api.clone())
            .unwrap()
            .iter_input_devices()
            .map(|i| DeviceInfo {
                name: i.name().to_string(),
                num_channels: i.input_channels as u16,
            })
            .collect()
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        Host::new(self.api.clone())
            .unwrap()
            .iter_output_devices()
            .map(|i| DeviceInfo {
                name: i.name().to_string(),
                num_channels: i.output_channels as u16,
            })
            .collect()
    }

    fn set_api(&mut self, name: &str) -> AudioDeviceResult<()> {
        self.api = rtaudio::compiled_apis()
            .iter()
            .find(|api| api.get_display_name().contains(name))
            .ok_or(AudioDeviceError::NotAvailable)?
            .clone();

        // update defaults
        let host = Host::new(self.api)?;
        self.input_device = host
            .iter_input_devices()
            .find(|d| d.is_default_input)
            .cloned()
            .unwrap();
        self.output_device = host
            .iter_output_devices()
            .find(|d| d.is_default_output)
            .cloned()
            .unwrap();

        Ok(())
    }

    fn set_input(&mut self, input: &str) -> AudioDeviceResult<()> {
        self.input_device = Host::new(self.api)?
            .iter_input_devices()
            .find(|device| device.name().contains(input))
            .ok_or(AudioDeviceError::NotAvailable)?
            .clone();
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> AudioDeviceResult<()> {
        self.output_device = Host::new(self.api)?
            .iter_output_devices()
            .find(|device| device.name().contains(output))
            .ok_or(AudioDeviceError::NotAvailable)?
            .clone();
        Ok(())
    }

    fn start(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> AudioDeviceResult<()> {
        self.stream_handle = Some(
            Host::new(self.api.clone())?
                .open_stream(&StreamConfig {
                    output_device: Some(DeviceParams {
                        device_id: Some(self.input_device.id.clone()),
                        num_channels: Some(config.num_input_channels as u32),
                        first_channel: 0,
                        fallback: true,
                        no_device_fallback: true,
                    }),
                    input_device: Some(DeviceParams {
                        device_id: Some(self.output_device.id.clone()),
                        num_channels: Some(config.num_output_channels as u32),
                        first_channel: 0,
                        fallback: true,
                        no_device_fallback: true,
                    }),
                    sample_format: rtaudio::SampleFormat::Float32,
                    sample_rate: Some(config.sample_rate),
                    buffer_frames: config.num_frames as u32,
                    flags: StreamFlags::empty(),
                    num_buffers: 2,
                    priority: -1,
                    name: String::new(),
                })
                .map_err(|(_, err)| err)?,
        );
        self.stream_handle
            .as_mut()
            .map(|handle| {
                handle.start(
                    move |buffers: rtaudio::Buffers<'_>,
                          info: &rtaudio::StreamInfo,
                          _status: rtaudio::StreamStatus| {
                        if let rtaudio::Buffers::Float32 { output, input } = buffers {
                            let input = Block::from_slice(input, info.in_channels as u16);
                            let output = BlockMut::from_slice(output, info.out_channels as u16);
                            process_fn(input, output);
                        }
                    },
                )
            })
            .transpose()?;

        Ok(())
    }

    fn stop(&mut self) -> AudioDeviceResult<()> {
        if let Some(mut stream_handle) = self.stream_handle.take() {
            stream_handle.stop();
        }
        Ok(())
    }
}
