use audio_host::{AudioBackend, AudioHost, Error};

fn main() -> Result<(), Error> {
    let device = AudioHost::new()?;

    // get available devices
    println!("APIs: {:#?}\n", device.apis());
    println!("Inputs: {:#?}\n", device.inputs());
    println!("Outputs: {:#?}\n", device.outputs());

    // get current selected devices
    println!("Selected API: {:#?}", device.api());
    println!("Selected Input: {:#?}", device.input());
    println!("Selected Output: {:#?}", device.output());

    Ok(())
}
