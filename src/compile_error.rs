#[cfg(not(any(feature = "cpal", feature = "rtaudio", feature = "juce",)))]
compile_error!("No audio backend selected. Please enable one of: rtaudio, cpal, juce, or web.");

#[cfg(all(feature = "cpal", feature = "rtaudio"))]
compile_error!("Audio backends are mutually exclusive. Please enable only one feature.");
#[cfg(all(feature = "juce", feature = "rtaudio"))]
compile_error!("Audio backends are mutually exclusive. Please enable only one feature.");
#[cfg(all(feature = "juce", feature = "cpal"))]
compile_error!("Audio backends are mutually exclusive. Please enable only one feature.");
