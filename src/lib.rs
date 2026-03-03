mod compile_error;

#[cfg(feature = "cpal")]
pub mod device_cpal;
#[cfg(feature = "juce")]
pub mod device_juce;
#[cfg(feature = "rtaudio")]
pub mod device_rtaudio;

#[cfg(feature = "cpal")]
pub use device_cpal::AudioDevice;
#[cfg(feature = "juce")]
pub use device_juce::AudioDevice;
#[cfg(feature = "rtaudio")]
pub use device_rtaudio::AudioDevice;

pub type AudioDeviceResult<T> = Result<T, Box<dyn std::error::Error>>;

pub type Block<'a> = InterleavedView<'a, f32>;
pub type BlockMut<'a> = InterleavedViewMut<'a, f32>;

pub use audio_blocks::*;

#[derive(thiserror::Error, Debug)]
pub enum AudioDeviceError {
    #[error("Wanted setting not available, leaving at default")]
    NotAvailable,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeviceInfo {
    pub name: String,
    pub num_channels: u16,
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    pub num_input_channels: u16,
    pub num_output_channels: u16,
    pub sample_rate: u32,
    pub num_frames: usize,
}

/// Trait defining the common interface for audio devices
pub trait AudioDeviceTrait {
    /// Create a new audio device with default settings
    fn new() -> AudioDeviceResult<Self>
    where
        Self: Sized;

    /// Get the current API/host name
    fn api(&self) -> String;

    /// Get all available APIs/hosts
    fn apis(&self) -> Vec<String>;

    /// Get the current input device name
    fn input(&self) -> String;

    /// Get the current output device name
    fn output(&self) -> String;

    /// Get all available input devices
    fn inputs(&self) -> Vec<DeviceInfo>;

    /// Get all available output devices
    fn outputs(&self) -> Vec<DeviceInfo>;

    /// Set the API/host by name
    fn set_api(&mut self, name: &str) -> AudioDeviceResult<()>;

    /// Set the input device by name
    fn set_input(&mut self, input: &str) -> AudioDeviceResult<()>;

    /// Set the output device by name
    fn set_output(&mut self, output: &str) -> AudioDeviceResult<()>;

    /// Start the audio stream with the given configuration and process callback
    fn start(
        &mut self,
        config: Config,
        process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> AudioDeviceResult<()>;

    /// Stop the audio stream
    fn stop(&mut self) -> AudioDeviceResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_device() {
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

        let num_frames = 1024;

        device
            .start(
                Config {
                    num_input_channels: 2,
                    num_output_channels: 2,
                    sample_rate: 48000,
                    num_frames,
                },
                move |input, mut output| {
                    if output.copy_from_block(&input).is_some() {
                        eprintln!("Input and Output buffer did not have a similar size");
                    }
                },
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(3));

        device.stop().unwrap();
    }
}
