use std::error::Error;

use audio_blocks::{AudioBlock, AudioBlockOps};
use audio_device::{AudioDevice, AudioDeviceTrait, Config};

fn main() -> Result<(), Box<dyn Error>> {
    let mut device = AudioDevice::new()?;

    // get available devices
    println!("{:#?}", device.apis());
    println!("{:#?}", device.inputs());
    println!("{:#?}", device.outputs());

    // get current selected devices
    println!("{:#?}", device.api());
    println!("{:#?}", device.input());
    println!("{:#?}", device.output());

    // select new devices
    device.set_api(&device.api()).unwrap();
    device.set_input(&device.input()).unwrap();
    device.set_output(&device.output()).unwrap();

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
                assert_eq!(input.num_frames(), 1024);
                assert_eq!(input.num_channels(), 2);

                assert_eq!(output.num_frames(), 1024);
                assert_eq!(output.num_channels(), 2);

                output.copy_from_block(&input);
            },
        )
        .unwrap();

    std::thread::sleep(std::time::Duration::from_secs(10));

    // stop audio device
    device.stop().unwrap();

    Ok(())
}
