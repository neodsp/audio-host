use std::fmt::Debug;

use audio_blocks::{AudioBlock, AudioBlockMut, Interleaved};
use cxx_juce::{
    JUCE,
    juce_audio_devices::{
        AudioCallbackHandle, AudioDeviceManager, AudioIODeviceCallback, AudioIODeviceType,
        ChannelCount, InputAudioSampleBuffer, OutputAudioSampleBuffer,
    },
};

use crate::{AudioBackend, Block, BlockMut, Config, DeviceInfo, Error};

pub struct AudioHost {
    _juce: JUCE,
    apis: Vec<String>,
    device_manager: AudioDeviceManager,
    input_device: String,
    output_device: String,
    handle: Option<AudioCallbackHandle>,
}

impl Debug for AudioHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioHost")
            .field("backend", &"JUCE")
            .field("is_running", &self.handle.is_some())
            .field("apis", &self.apis())
            .field("inputs", &self.inputs())
            .field("outputs", &self.outputs())
            .finish()
    }
}

impl AudioBackend for AudioHost {
    fn new() -> Result<Self, Error> {
        let juce = JUCE::initialise();
        let mut device_manager = AudioDeviceManager::new(&juce);
        device_manager
            .initialise(256, 256)
            .map_err(|e| Error::Backend(Box::new(e)))?;
        let mut apis = Vec::new();
        for api in device_manager.device_types() {
            apis.push(api.name().to_string());
        }
        Ok(Self {
            _juce: juce,
            apis,
            input_device: device_manager
                .audio_device_setup()
                .input_device_name()
                .into(),
            output_device: device_manager
                .audio_device_setup()
                .output_device_name()
                .into(),
            device_manager,
            handle: None,
        })
    }

    fn api(&self) -> String {
        let device_type = self.device_manager.current_device_type().unwrap();
        device_type.name()
    }

    fn apis(&self) -> Vec<String> {
        self.apis.clone()
    }

    fn input(&self) -> String {
        self.device_manager
            .audio_device_setup()
            .input_device_name()
            .into()
    }

    fn output(&self) -> String {
        self.device_manager
            .audio_device_setup()
            .output_device_name()
            .into()
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        let device_type = self.device_manager.current_device_type().unwrap();
        device_type
            .input_devices()
            .iter()
            .map(|d: &String| DeviceInfo {
                name: d.clone(),
                num_channels: 0,
            })
            .collect()
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        let device_type = self.device_manager.current_device_type().unwrap();
        device_type
            .output_devices()
            .iter()
            .map(|d: &String| DeviceInfo {
                name: d.clone(),
                num_channels: 0,
            })
            .collect()
    }

    fn set_api(&mut self, name: &str) -> Result<(), Error> {
        self.device_manager.set_current_audio_device_type(name);
        // update setup
        self.input_device = self.input();
        self.output_device = self.output();
        Ok(())
    }

    fn set_input(&mut self, input: &str) -> Result<(), Error> {
        let device = self
            .inputs()
            .iter().find(|&p| p.name.contains(input)).cloned()
            .ok_or(Error::NotFound)?;
        self.input_device = device.name.clone();
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> Result<(), Error> {
        let device = self
            .outputs()
            .iter().find(|&p| p.name.contains(output)).cloned()
            .ok_or(Error::NotFound)?;
        self.output_device = device.name.clone();
        Ok(())
    }

    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error> {
        self.stop()?;
        config.validate()?;

        let mut setup = self.device_manager.audio_device_setup();
        setup = setup.with_input_channels(ChannelCount::Custom(config.num_input_channels as i32));
        setup = setup.with_output_channels(ChannelCount::Custom(config.num_output_channels as i32));
        setup = setup.with_sample_rate(config.sample_rate as f64);
        setup = setup.with_buffer_size(config.num_frames);

        self.device_manager.set_audio_device_setup(&setup);

        self.handle = Some(
            self.device_manager
                .add_audio_callback(AudioCallback::new(process_fn)),
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), Error> {
        if let Some(handle) = self.handle.take() {
            self.device_manager.remove_audio_callback(handle);
        }
        Ok(())
    }
}

struct AudioCallback<F: FnMut(Block, BlockMut) + Send + 'static> {
    process_fn: F,
    input_block: Interleaved<f32>,
    output_block: Interleaved<f32>,
}

impl<F: FnMut(Block, BlockMut) + Send + 'static> AudioCallback<F> {
    pub fn new(process_fn: F) -> Self {
        Self {
            process_fn,
            input_block: Interleaved::new(0, 0),
            output_block: Interleaved::new(0, 0),
        }
    }
}

impl<F: FnMut(Block, BlockMut) + Send + 'static> AudioIODeviceCallback for AudioCallback<F> {
    fn about_to_start(&mut self, device: &mut dyn cxx_juce::juce_audio_devices::AudioIODevice) {
        let num_input_channels = device.input_channels() as u16;
        let num_output_channels = device.output_channels() as u16;
        let num_frames = device.buffer_size();
        self.input_block = Interleaved::new(num_input_channels, num_frames);
        self.output_block = Interleaved::new(num_output_channels, num_frames);
    }

    fn process_block(
        &mut self,
        input: &InputAudioSampleBuffer,
        output: &mut OutputAudioSampleBuffer,
    ) {
        // resize buffers
        self.input_block
            .set_visible(input.channels() as u16, input.samples());
        self.output_block
            .set_visible(output.channels() as u16, output.samples());

        // copy input
        for ch in 0..input.channels() {
            let channel = &input[ch];
            for frame in 0..input.samples() {
                *self.input_block.sample_mut(ch as u16, frame) = channel[frame];
            }
        }

        // user process
        (self.process_fn)(self.input_block.view(), self.output_block.view_mut());

        // copy output
        let num_samples = output.samples();
        for ch in 0..output.channels() {
            let channel = &mut output[ch];
            for frame in 0..num_samples {
                channel[frame] = self.output_block.sample(ch as u16, frame);
            }
        }
    }

    fn stopped(&mut self) {}
}
