use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use audio_host::{AudioBackend, AudioBlockOpsMut, AudioHost, Config, Error};

fn main() -> Result<(), Error> {
    let mut host = AudioHost::new()?;

    // the audio callback must not block (no printing, locking or allocating), so it only
    // counts problems and they are reported after the stream stops
    let mismatched_blocks = Arc::new(AtomicUsize::new(0));
    let mismatched = mismatched_blocks.clone();

    // start audio host
    let config = Config::default()
        .with_input_channels(2)
        .with_output_channels(2);
    host.start(config, move |input, mut output| {
        if output.copy_from_block(&input).is_some() {
            mismatched.fetch_add(1, Ordering::Relaxed);
        }
    })?;

    std::thread::sleep(std::time::Duration::from_secs(10));

    // stop audio host
    host.stop()?;

    let mismatched = mismatched_blocks.load(Ordering::Relaxed);
    if mismatched > 0 {
        eprintln!("{mismatched} input and output blocks did not have the same size");
    }

    Ok(())
}
