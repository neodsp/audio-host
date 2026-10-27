use std::fmt::Debug;

use audio_blocks::{AudioBlock, AudioBlockMut, Interleaved};
use cxx_juce::{
    JUCE,
    juce_audio_devices::{
        AudioCallbackHandle, AudioDeviceManager, AudioIODevice, AudioIODeviceCallback,
        AudioIODeviceType, ChannelCount, InputAudioSampleBuffer, OutputAudioSampleBuffer,
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
        let mut apis = Vec::new();
        for api in device_manager.device_types() {
            apis.push(api.name().to_string());
        }
        let mut host = Self {
            _juce: juce,
            apis,
            device_manager,
            input_device: String::new(),
            output_device: String::new(),
            handle: None,
        };
        host.select_default_devices()?;
        Ok(host)
    }

    fn api(&self) -> String {
        let device_type = self.device_manager.current_device_type().unwrap();
        device_type.name()
    }

    fn apis(&self) -> Vec<String> {
        self.apis.clone()
    }

    fn input(&self) -> String {
        self.input_device.clone()
    }

    fn output(&self) -> String {
        self.output_device.clone()
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
        let api = self
            .apis
            .iter()
            .find(|api| api.contains(name))
            .cloned()
            .ok_or(Error::NotFound)?;

        // JUCE would silently pick another API if this one has no devices
        let has_devices = self
            .device_manager
            .device_types()
            .iter()
            .find(|t| t.name() == api)
            .is_some_and(|t| !t.input_devices().is_empty() || !t.output_devices().is_empty());
        if !has_devices {
            return Err(Error::NotFound);
        }

        self.stop()?;
        self.device_manager.set_current_audio_device_type(&api);
        // switching the API resets the selection to that API's default devices
        self.select_default_devices()
    }

    fn set_input(&mut self, input: &str) -> Result<(), Error> {
        let device = self
            .inputs()
            .iter()
            .find(|&p| p.name.contains(input))
            .cloned()
            .ok_or(Error::NotFound)?;
        self.input_device = device.name.clone();
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> Result<(), Error> {
        let device = self
            .outputs()
            .iter()
            .find(|&p| p.name.contains(output))
            .cloned()
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

        if config.num_input_channels > 0 && self.input_device.is_empty() {
            return Err(Error::UnsupportedInputChannels {
                requested: config.num_input_channels,
                supported: 0,
            });
        }
        if config.num_output_channels > 0 && self.output_device.is_empty() {
            return Err(Error::UnsupportedOutputChannels {
                requested: config.num_output_channels,
                supported: 0,
            });
        }

        // only open the devices that are actually used
        let input_device = if config.num_input_channels > 0 {
            self.input_device.clone()
        } else {
            String::new()
        };
        let output_device = if config.num_output_channels > 0 {
            self.output_device.clone()
        } else {
            String::new()
        };

        let mut setup = self.device_manager.audio_device_setup();
        setup = setup.with_input_device_name(&input_device);
        setup = setup.with_output_device_name(&output_device);
        setup = setup.with_input_channels(ChannelCount::Custom(config.num_input_channels as i32));
        setup = setup.with_output_channels(ChannelCount::Custom(config.num_output_channels as i32));
        setup = setup.with_sample_rate(config.sample_rate as f64);
        setup = setup.with_buffer_size(config.num_frames);

        self.device_manager.set_audio_device_setup(&setup);

        // cxx-juce drops JUCE's error message, but JUCE clears the device names when opening fails
        let actual = self.device_manager.audio_device_setup();
        if actual.input_device_name() != input_device
            || actual.output_device_name() != output_device
        {
            self.close_device();
            return Err(Error::Backend(
                format!("could not open the audio device (input: '{input_device}', output: '{output_device}')")
                    .into(),
            ));
        }

        // JUCE silently lowers the channel count and picks the nearest sample rate,
        // so compare what was opened with what was requested
        let (num_input_channels, num_output_channels, sample_rate, available_rates) = {
            let Some(mut device) = self.device_manager.current_device() else {
                return Err(Error::Backend("no audio device was opened".into()));
            };
            let available_rates: Vec<u32> = device
                .available_sample_rates()
                .into_iter()
                .map(|rate| rate as u32)
                .collect();
            (
                device.input_channels().max(0) as u16,
                device.output_channels().max(0) as u16,
                device.sample_rate(),
                available_rates,
            )
        };

        let error = if num_input_channels < config.num_input_channels {
            Some(Error::UnsupportedInputChannels {
                requested: config.num_input_channels,
                supported: num_input_channels,
            })
        } else if num_output_channels < config.num_output_channels {
            Some(Error::UnsupportedOutputChannels {
                requested: config.num_output_channels,
                supported: num_output_channels,
            })
        } else if (sample_rate - config.sample_rate as f64).abs() > 0.5 {
            Some(Error::UnsupportedSampleRate {
                requested: config.sample_rate,
                available: available_rates,
            })
        } else {
            None
        };
        if let Some(error) = error {
            self.close_device();
            return Err(error);
        }

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
        // release the devices, like the other backends do
        self.close_device();
        Ok(())
    }
}

impl AudioHost {
    /// Selects the default devices of the current API. JUCE only reports them by opening
    /// them, so they are opened briefly and closed again.
    fn select_default_devices(&mut self) -> Result<(), Error> {
        self.device_manager
            .initialise(256, 256)
            .map_err(|e| Error::Backend(Box::new(e)))?;
        let setup = self.device_manager.audio_device_setup();
        self.input_device = setup.input_device_name().into();
        self.output_device = setup.output_device_name().into();
        self.close_device();
        Ok(())
    }

    /// Closes the current device. JUCE does this when no device names are set.
    fn close_device(&mut self) {
        let setup = self
            .device_manager
            .audio_device_setup()
            .with_input_device_name("")
            .with_output_device_name("");
        self.device_manager.set_audio_device_setup(&setup);
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
    fn about_to_start(&mut self, device: &mut dyn AudioIODevice) {
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
            #[allow(clippy::needless_range_loop)]
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
            #[allow(clippy::needless_range_loop)]
            for frame in 0..num_samples {
                channel[frame] = self.output_block.sample(ch as u16, frame);
            }
        }
    }

    fn stopped(&mut self) {}
}
