use audio_host::{AudioBackend, AudioBlockOpsMut, AudioHost, Config, Error};

fn main() -> Result<(), Error> {
    let mut host = AudioHost::new()?;

    // start audio host
    host.start(Config::default(), move |input, mut output| {
        if output.copy_from_block(&input).is_some() {
            eprintln!("Input and Output buffer did not have a similar size");
        }
    })
    .unwrap();

    std::thread::sleep(std::time::Duration::from_secs(10));

    // stop audio host
    host.stop().unwrap();

    Ok(())
}
