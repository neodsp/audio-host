use std::{fmt::Debug, sync::Arc};

use audio_blocks::{AudioBlock, AudioBlockMut, Interleaved};
use cxx_juce::{
    JUCE,
    juce_audio_devices::{
        AudioCallbackHandle, AudioDeviceManager, AudioIODevice, AudioIODeviceCallback,
        AudioIODeviceType, ChannelCount, InputAudioSampleBuffer, OutputAudioSampleBuffer,
    },
};

use crate::{
    AudioBackend, Block, BlockMut, Config, DeviceInfo, Error, StatusTracker, StreamError,
    StreamStatus, find_by_name,
};

/// Audio host using JUCE.
///
/// JUCE can only report its default devices by opening them. To avoid opening devices that
/// aren't used (and e.g. triggering the microphone permission prompt on macOS), the default
/// devices are chosen in [`start`](AudioBackend::start), only for the directions the config
/// uses. Until then [`input`](AudioBackend::input) and [`output`](AudioBackend::output) return
/// an empty string if no device was selected.
pub struct AudioHost {
    _juce: JUCE,
    apis: Vec<String>,
    device_manager: AudioDeviceManager,
    /// Selected with `set_input`, `None` means JUCE's default device.
    input_device: Option<String>,
    /// Selected with `set_output`, `None` means JUCE's default device.
    output_device: Option<String>,
    /// The default input device JUCE picked in the last `start`, empty if unknown.
    default_input: String,
    /// The default output device JUCE picked in the last `start`, empty if unknown.
    default_output: String,
    handle: Option<AudioCallbackHandle>,
    status: Arc<StatusTracker>,
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
        // picks the API without opening any device (no channels are needed yet)
        device_manager
            .initialise(0, 0)
            .map_err(|e| Error::Backend(Box::new(e)))?;
        Ok(Self {
            _juce: juce,
            apis,
            device_manager,
            input_device: None,
            output_device: None,
            default_input: String::new(),
            default_output: String::new(),
            handle: None,
            status: Arc::default(),
        })
    }

    fn api(&self) -> String {
        self.device_manager
            .current_device_type()
            .map(|t| t.name())
            .unwrap_or_default()
    }

    fn apis(&self) -> Vec<String> {
        self.apis.clone()
    }

    fn input(&self) -> String {
        self.input_device
            .clone()
            .unwrap_or_else(|| self.default_input.clone())
    }

    fn output(&self) -> String {
        self.output_device
            .clone()
            .unwrap_or_else(|| self.default_output.clone())
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        let Some(device_type) = self.device_manager.current_device_type() else {
            return Vec::new();
        };
        device_type
            .input_devices()
            .into_iter()
            .map(|name| DeviceInfo {
                name,
                // cxx-juce can't report channel counts without opening the device
                num_channels: None,
            })
            .collect()
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        let Some(device_type) = self.device_manager.current_device_type() else {
            return Vec::new();
        };
        device_type
            .output_devices()
            .into_iter()
            .map(|name| DeviceInfo {
                name,
                // cxx-juce can't report channel counts without opening the device
                num_channels: None,
            })
            .collect()
    }

    fn set_api(&mut self, name: &str) -> Result<(), Error> {
        let api = find_by_name(self.apis.iter(), |api| Some(api.to_string()), name)
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
        // no channels needed, so switching doesn't open the new API's default devices. It still
        // reopens the devices last used with that API, if any, so close them right away.
        self.device_manager
            .initialise(0, 0)
            .map_err(|e| Error::Backend(Box::new(e)))?;
        self.device_manager.set_current_audio_device_type(&api);
        self.close_device();

        // switching the API resets the selection to that API's default devices
        self.input_device = None;
        self.output_device = None;
        self.default_input.clear();
        self.default_output.clear();
        Ok(())
    }

    fn set_input(&mut self, input: &str) -> Result<(), Error> {
        let device =
            find_by_name(self.inputs(), |d| Some(d.name.clone()), input).ok_or(Error::NotFound)?;
        self.input_device = Some(device.name);
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> Result<(), Error> {
        let device = find_by_name(self.outputs(), |d| Some(d.name.clone()), output)
            .ok_or(Error::NotFound)?;
        self.output_device = Some(device.name);
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

        // let JUCE pick its default devices, only for the directions that are used
        let default_input_channels = match self.input_device {
            None => config.num_input_channels,
            Some(_) => 0,
        };
        let default_output_channels = match self.output_device {
            None => config.num_output_channels,
            Some(_) => 0,
        };
        if default_input_channels > 0 || default_output_channels > 0 {
            self.device_manager
                .initialise(
                    default_input_channels as usize,
                    default_output_channels as usize,
                )
                .map_err(|e| Error::Backend(Box::new(e)))?;
            let setup = self.device_manager.audio_device_setup();
            if default_input_channels > 0 {
                self.default_input = setup.input_device_name().into();
            }
            if default_output_channels > 0 {
                self.default_output = setup.output_device_name().into();
            }
        }

        if config.num_input_channels > 0 && self.input().is_empty() {
            self.close_device();
            return Err(Error::UnsupportedInputChannels {
                requested: config.num_input_channels,
                supported: 0,
            });
        }
        if config.num_output_channels > 0 && self.output().is_empty() {
            self.close_device();
            return Err(Error::UnsupportedOutputChannels {
                requested: config.num_output_channels,
                supported: 0,
            });
        }

        // only open the devices that are actually used
        let input_device = if config.num_input_channels > 0 {
            self.input()
        } else {
            String::new()
        };
        let output_device = if config.num_output_channels > 0 {
            self.output()
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

        self.status.set_running(true);
        self.handle = Some(
            self.device_manager
                .add_audio_callback(AudioCallback::new(process_fn, self.status.clone())),
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), Error> {
        // not running first, so the callback's `stopped` isn't reported as an error
        self.status.set_running(false);
        if let Some(handle) = self.handle.take() {
            self.device_manager.remove_audio_callback(handle);
        }
        // release the devices, like the other backends do
        self.close_device();
        Ok(())
    }

    fn status(&self) -> StreamStatus {
        self.status.status()
    }
}

impl AudioHost {
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
    status: Arc<StatusTracker>,
}

impl<F: FnMut(Block, BlockMut) + Send + 'static> AudioCallback<F> {
    pub fn new(process_fn: F, status: Arc<StatusTracker>) -> Self {
        Self {
            process_fn,
            input_block: Interleaved::new(0, 0),
            output_block: Interleaved::new(0, 0),
            status,
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

    fn stopped(&mut self) {
        // `stop` marks the stream as not running first, so this is the device stopping on its
        // own. cxx-juce doesn't pass on JUCE's error message.
        self.status.fail(StreamError::Backend(
            "audio device stopped unexpectedly".into(),
        ));
    }
}
