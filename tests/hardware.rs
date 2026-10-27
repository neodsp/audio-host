//! Tests against the real audio devices of this machine. They are ignored by default, run them
//! manually with:
//!
//! ```sh
//! cargo test --test hardware -- --ignored
//! ```
//!
//! All callbacks write silence. Tests that need an input device are skipped without one.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use audio_host::{AudioBackend, AudioBlock, AudioBlockOpsMut, AudioHost, Config, Error};

/// Only one test may use the audio devices at a time.
static DEVICES: Mutex<()> = Mutex::new(());

fn lock_devices() -> std::sync::MutexGuard<'static, ()> {
    DEVICES.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the process callback saw while the stream was running.
#[derive(Debug)]
struct Run {
    callbacks: usize,
    input_channels: usize,
    output_channels: usize,
    /// Duplex callbacks where input and output blocks had a different number of frames.
    mismatched_blocks: usize,
}

/// Runs a silent stream for half a second.
fn run(host: &mut AudioHost, config: Config) -> Result<Run, Error> {
    let callbacks = Arc::new(AtomicUsize::new(0));
    let input_channels = Arc::new(AtomicUsize::new(0));
    let output_channels = Arc::new(AtomicUsize::new(0));
    let mismatched_blocks = Arc::new(AtomicUsize::new(0));
    let (c, i, o, m) = (
        callbacks.clone(),
        input_channels.clone(),
        output_channels.clone(),
        mismatched_blocks.clone(),
    );
    let duplex = config.num_input_channels > 0 && config.num_output_channels > 0;

    host.start(config, move |input, mut output| {
        output.clear();
        c.fetch_add(1, Ordering::Relaxed);
        i.store(input.num_channels() as usize, Ordering::Relaxed);
        o.store(output.num_channels() as usize, Ordering::Relaxed);
        if duplex && input.num_frames() != output.num_frames() {
            m.fetch_add(1, Ordering::Relaxed);
        }
    })?;
    std::thread::sleep(Duration::from_millis(500));
    host.stop().expect("stop failed");

    Ok(Run {
        callbacks: callbacks.load(Ordering::Relaxed),
        input_channels: input_channels.load(Ordering::Relaxed),
        output_channels: output_channels.load(Ordering::Relaxed),
        mismatched_blocks: mismatched_blocks.load(Ordering::Relaxed),
    })
}

fn has_input(host: &AudioHost) -> bool {
    // JUCE only names its default device after start(), so also check the device list
    if host.input().is_empty() && host.inputs().is_empty() {
        eprintln!("skipped: no input device");
        return false;
    }
    true
}

#[test]
#[ignore = "needs audio hardware"]
fn output_only_stream_runs() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();

    let run = run(&mut host, Config::default()).unwrap();
    assert!(run.callbacks > 0, "process callback was never called");
    assert_eq!(run.output_channels, 2);
}

#[test]
#[ignore = "needs audio hardware"]
fn input_only_stream_runs() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();
    if !has_input(&host) {
        return;
    }

    let config = Config {
        num_input_channels: 1,
        num_output_channels: 0,
        ..Config::default()
    };
    let run = run(&mut host, config).unwrap();
    assert!(run.callbacks > 0, "process callback was never called");
    assert_eq!(run.input_channels, 1);
}

#[test]
#[ignore = "needs audio hardware"]
fn duplex_stream_runs() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();
    if !has_input(&host) {
        return;
    }

    let config = Config {
        num_input_channels: 1,
        num_output_channels: 2,
        ..Config::default()
    };
    let run = run(&mut host, config).unwrap();
    assert!(run.callbacks > 0, "process callback was never called");
    assert_eq!(run.input_channels, 1);
    assert_eq!(run.output_channels, 2);
    assert_eq!(
        run.mismatched_blocks, 0,
        "input and output blocks differ in length"
    );
}

#[test]
#[ignore = "needs audio hardware"]
fn too_many_output_channels_reports_a_working_count() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();

    let config = Config {
        num_output_channels: 1000,
        ..Config::default()
    };
    let supported = match run(&mut host, config.clone()) {
        Err(Error::UnsupportedOutputChannels {
            requested,
            supported,
        }) => {
            assert_eq!(requested, 1000);
            supported
        }
        other => panic!("expected UnsupportedOutputChannels, got {other:?}"),
    };
    assert!(supported > 0 && supported < 1000);

    // the device list reports the same maximum (if the backend knows it)
    let listed = host.outputs().into_iter().find(|d| d.name == host.output());
    if let Some(num_channels) = listed.and_then(|d| d.num_channels) {
        assert_eq!(num_channels, supported);
    }

    // retrying with the reported count works
    let run = run(
        &mut host,
        Config {
            num_output_channels: supported,
            ..config
        },
    )
    .unwrap();
    assert_eq!(run.output_channels, supported as usize);
}

#[test]
#[ignore = "needs audio hardware"]
fn too_many_input_channels_reports_a_working_count() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();
    if !has_input(&host) {
        return;
    }

    let config = Config {
        num_input_channels: 1000,
        ..Config::default()
    };
    let supported = match run(&mut host, config.clone()) {
        Err(Error::UnsupportedInputChannels {
            requested,
            supported,
        }) => {
            assert_eq!(requested, 1000);
            supported
        }
        other => panic!("expected UnsupportedInputChannels, got {other:?}"),
    };
    assert!(supported > 0 && supported < 1000);

    // the device list reports the same maximum (if the backend knows it)
    let listed = host.inputs().into_iter().find(|d| d.name == host.input());
    if let Some(num_channels) = listed.and_then(|d| d.num_channels) {
        assert_eq!(num_channels, supported);
    }

    let run = run(
        &mut host,
        Config {
            num_input_channels: supported,
            ..config
        },
    )
    .unwrap();
    assert_eq!(run.input_channels, supported as usize);
}

#[test]
#[ignore = "needs audio hardware"]
fn unusual_sample_rate_runs_or_reports_working_rates() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();

    let config = Config {
        sample_rate: 12345,
        ..Config::default()
    };
    match run(&mut host, config.clone()) {
        // some devices resample, then the rate really is supported
        Ok(run) => assert!(run.callbacks > 0),
        Err(Error::UnsupportedSampleRate {
            requested,
            available,
        }) => {
            assert_eq!(requested, 12345);
            assert!(!available.contains(&12345));
            // retrying with a reported rate works
            if let Some(&rate) = available.first() {
                run(
                    &mut host,
                    Config {
                        sample_rate: rate,
                        ..config
                    },
                )
                .unwrap();
            }
        }
        Err(other) => panic!("expected Ok or UnsupportedSampleRate, got {other:?}"),
    }
}

#[test]
#[ignore = "needs audio hardware"]
fn status_reports_running_and_stopped() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();
    assert!(!host.status().running);

    host.start(Config::default(), |_input, mut output| output.clear())
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let status = host.status();
    assert!(status.running, "{status:?}");
    assert_eq!(status.error, None);
    if status.xruns > 0 {
        eprintln!("note: {} xruns while running", status.xruns);
    }

    host.stop().unwrap();
    let status = host.status();
    assert!(!status.running);
    assert_eq!(
        status.error, None,
        "stop() must not be reported as an error"
    );
}

#[test]
#[ignore = "needs audio hardware"]
fn stream_can_be_restarted() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();

    host.stop().expect("stop without start should work");
    for _ in 0..2 {
        let run = run(&mut host, Config::default()).unwrap();
        assert!(run.callbacks > 0);
    }
}

#[test]
#[ignore = "needs audio hardware"]
fn selecting_apis_and_devices() {
    let _lock = lock_devices();
    let mut host = AudioHost::new().unwrap();

    let api = host.api();
    assert!(host.apis().contains(&api));
    host.set_api(&api).unwrap();

    assert!(matches!(
        host.set_api("no such api 1234"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        host.set_input("no such device 1234"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        host.set_output("no such device 1234"),
        Err(Error::NotFound)
    ));

    // the default output may be a virtual device that is not listed (e.g. cpal on Linux)
    if let Some(output) = host.outputs().first() {
        host.set_output(&output.name).unwrap();
        assert!(host.output().contains(&output.name));
    }
}
