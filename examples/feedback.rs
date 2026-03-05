use audio_io::{AudioBlockOpsMut, AudioHost, AudioHostError, AudioHostTrait, Config};

fn main() -> Result<(), AudioHostError> {
    let mut device = AudioHost::new()?;

    // start audio device
    device
        .start(Config::default(), move |input, mut output| {
            if output.copy_from_block(&input).is_some() {
                eprintln!("Input and Output buffer did not have a similar size");
            }
        })
        .unwrap();

    std::thread::sleep(std::time::Duration::from_secs(10));

    // stop audio device
    device.stop().unwrap();

    Ok(())
}
