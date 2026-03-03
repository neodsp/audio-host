use audio_io::{AudioBlockOpsMut, AudioDevice, AudioDeviceResult, AudioDeviceTrait, Config};

fn main() -> AudioDeviceResult<()> {
    let mut device = AudioDevice::new()?;

    // start audio device
    device
        .start(
            Config {
                num_input_channels: 2,
                num_output_channels: 2,
                sample_rate: 48000,
                num_frames: 1024,
            },
            move |input, mut output| {
                if output.copy_from_block(&input).is_some() {
                    eprintln!("Input and Output buffer did not have a similar size");
                }
            },
        )
        .unwrap();

    std::thread::sleep(std::time::Duration::from_secs(10));

    // stop audio device
    device.stop().unwrap();

    Ok(())
}
