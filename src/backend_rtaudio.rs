use std::{
    cell::Cell,
    fmt::Debug,
    sync::{Arc, Mutex, Once, Weak},
};

use rtaudio::{
    DeviceParams, Host, RtAudioError, RtAudioErrorType, StreamConfig, StreamFlags, StreamHandle,
};

use crate::{
    AudioBackend, Block, BlockMut, Config, DeviceInfo, Error, StatusTracker, StreamError,
    StreamStatus, find_by_name,
};

/// Audio host using rtaudio.
///
/// Stream errors come from rtaudio's error callback, which is shared by the whole process. Don't
/// replace it with `rtaudio::set_error_callback`, or `status()` stops reporting errors. rtaudio
/// doesn't say which stream failed, so an error stops the status of every running host.
pub struct AudioHost {
    api: rtaudio::Api,
    input_device: Option<rtaudio::DeviceInfo>,
    output_device: Option<rtaudio::DeviceInfo>,
    stream_handle: Option<StreamHandle>,
    status: Arc<StatusTracker>,
}

impl Drop for AudioHost {
    fn drop(&mut self) {
        // close the stream through `stop`, so errors while closing aren't reported to other hosts
        let _ = self.stop();
    }
}

impl Debug for AudioHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioHost")
            .field("backend", &"RtAudio")
            .field("is_running", &self.stream_handle.is_some())
            // only the current selection: listing every device is slow and opens them
            .field("api", &self.api())
            .field("input", &self.input())
            .field("output", &self.output())
            .finish()
    }
}

impl AudioBackend for AudioHost {
    fn new() -> Result<Self, Error> {
        let _api_call = ApiCall::enter();
        let host = Host::default();
        let input_device = host
            .iter_input_devices()
            .find(|d| d.is_default_input)
            .cloned();
        let output_device = host
            .iter_output_devices()
            .find(|d| d.is_default_output)
            .cloned();
        Ok(Self {
            api: host.api(),
            input_device,
            output_device,
            stream_handle: None,
            status: register_status(),
        })
    }

    fn api(&self) -> String {
        self.api.get_display_name()
    }

    fn apis(&self) -> Vec<String> {
        rtaudio::compiled_apis()
            .iter()
            .map(|a| a.get_display_name())
            .collect()
    }

    fn input(&self) -> String {
        self.input_device
            .as_ref()
            .map_or(String::new(), |d| d.name().to_string())
    }

    fn output(&self) -> String {
        self.output_device
            .as_ref()
            .map_or(String::new(), |d| d.name().to_string())
    }

    fn inputs(&self) -> Vec<DeviceInfo> {
        let _api_call = ApiCall::enter();
        let Ok(host) = Host::new(self.api) else {
            return Vec::new();
        };
        host.iter_input_devices()
            .map(|i| DeviceInfo {
                name: i.name().to_string(),
                num_channels: Some(to_u16(i.input_channels)),
            })
            .collect()
    }

    fn outputs(&self) -> Vec<DeviceInfo> {
        let _api_call = ApiCall::enter();
        let Ok(host) = Host::new(self.api) else {
            return Vec::new();
        };
        host.iter_output_devices()
            .map(|i| DeviceInfo {
                name: i.name().to_string(),
                num_channels: Some(to_u16(i.output_channels)),
            })
            .collect()
    }

    fn set_api(&mut self, name: &str) -> Result<(), Error> {
        let _api_call = ApiCall::enter();
        let api = find_by_name(
            rtaudio::compiled_apis().iter().copied(),
            |api| Some(api.get_display_name()),
            name,
        )
        .ok_or(Error::NotFound)?;

        // nothing changes unless the new API can be opened
        let host = Host::new(api).map_err(|e| Error::Backend(Box::new(e)))?;

        // a running stream belongs to the old API, so stop it (like the JUCE backend)
        self.stop()?;
        self.api = api;
        self.input_device = host
            .iter_input_devices()
            .find(|d| d.is_default_input)
            .cloned();
        self.output_device = host
            .iter_output_devices()
            .find(|d| d.is_default_output)
            .cloned();

        Ok(())
    }

    fn set_input(&mut self, input: &str) -> Result<(), Error> {
        let _api_call = ApiCall::enter();
        let host = Host::new(self.api).map_err(|e| Error::Backend(Box::new(e)))?;
        let device = find_by_name(
            host.iter_input_devices(),
            |d| Some(d.name().to_string()),
            input,
        )
        .ok_or(Error::NotFound)?;
        self.input_device = Some(device.clone());
        Ok(())
    }

    fn set_output(&mut self, output: &str) -> Result<(), Error> {
        let _api_call = ApiCall::enter();
        let host = Host::new(self.api).map_err(|e| Error::Backend(Box::new(e)))?;
        let device = find_by_name(
            host.iter_output_devices(),
            |d| Some(d.name().to_string()),
            output,
        )
        .ok_or(Error::NotFound)?;
        self.output_device = Some(device.clone());
        Ok(())
    }

    fn start(
        &mut self,
        config: Config,
        mut process_fn: impl FnMut(Block, BlockMut) + Send + 'static,
    ) -> Result<(), Error> {
        let _api_call = ApiCall::enter();
        self.stop()?;
        config.validate()?;
        self.status.reset();

        // check the request up front, so a running stream always matches the config
        let input_device = if config.num_input_channels > 0 {
            let available = self.input_device.as_ref().map_or(0, |d| d.input_channels);
            if available < config.num_input_channels as u32 {
                return Err(Error::UnsupportedInputChannels {
                    requested: config.num_input_channels,
                    supported: to_u16(available),
                });
            }
            self.input_device.as_ref()
        } else {
            None
        };

        let output_device = if config.num_output_channels > 0 {
            let available = self.output_device.as_ref().map_or(0, |d| d.output_channels);
            if available < config.num_output_channels as u32 {
                return Err(Error::UnsupportedOutputChannels {
                    requested: config.num_output_channels,
                    supported: to_u16(available),
                });
            }
            self.output_device.as_ref()
        } else {
            None
        };

        // sample rates supported by all used devices (devices that report none are skipped)
        let mut available_rates: Option<Vec<u32>> = None;
        for device in [input_device, output_device].into_iter().flatten() {
            if device.sample_rates.is_empty() {
                continue;
            }
            available_rates = Some(match available_rates {
                None => device.sample_rates.clone(),
                Some(rates) => rates
                    .into_iter()
                    .filter(|rate| device.sample_rates.contains(rate))
                    .collect(),
            });
        }
        if let Some(available) = &available_rates
            && !available.contains(&config.sample_rate)
        {
            return Err(Error::UnsupportedSampleRate {
                requested: config.sample_rate,
                available: available.clone(),
            });
        }

        // no fallback: opening a different device than the selected one would break the config
        let input_params = input_device.map(|d| DeviceParams {
            device_id: Some(d.id.clone()),
            num_channels: Some(config.num_input_channels as u32),
            first_channel: 0,
            fallback: false,
            no_device_fallback: true,
        });

        let output_params = output_device.map(|d| DeviceParams {
            device_id: Some(d.id.clone()),
            num_channels: Some(config.num_output_channels as u32),
            first_channel: 0,
            fallback: false,
            no_device_fallback: true,
        });

        let mut stream_handle = Host::new(self.api)
            .map_err(|e| Error::Backend(Box::new(e)))?
            .open_stream(&StreamConfig {
                input_device: input_params,
                output_device: output_params,
                sample_format: rtaudio::SampleFormat::Float32,
                sample_rate: Some(config.sample_rate),
                buffer_frames: config.num_frames as u32,
                flags: StreamFlags::empty(),
                num_buffers: 2,
                priority: -1,
                name: String::new(),
            })
            .map_err(|(_, err)| Error::Backend(Box::new(err)))?;

        // double check what was opened (the stream is closed when the handle is dropped)
        let info = stream_handle.info();
        // if the selected device is gone, rtaudio opens the first device with the same name
        // instead, even without fallback
        if info.input_device.as_ref() != input_device.map(|d| &d.id)
            || info.output_device.as_ref() != output_device.map(|d| &d.id)
        {
            return Err(Error::NotFound);
        }
        if info.in_channels != config.num_input_channels as usize {
            return Err(Error::UnsupportedInputChannels {
                requested: config.num_input_channels,
                supported: to_u16(info.in_channels as u32),
            });
        }
        if info.out_channels != config.num_output_channels as usize {
            return Err(Error::UnsupportedOutputChannels {
                requested: config.num_output_channels,
                supported: to_u16(info.out_channels as u32),
            });
        }
        if info.sample_rate != config.sample_rate {
            return Err(Error::UnsupportedSampleRate {
                requested: config.sample_rate,
                available: rates_without(available_rates, config.sample_rate, info.sample_rate),
            });
        }

        let status = self.status.clone();
        self.status.set_running(true);
        let result = stream_handle.start(
            move |buffers: rtaudio::Buffers<'_>,
                  info: &rtaudio::StreamInfo,
                  stream_status: rtaudio::StreamStatus| {
                // input overflow or output underflow since the last callback
                if !stream_status.is_empty() {
                    status.add_xrun();
                }
                if let rtaudio::Buffers::Float32 { output, input } = buffers {
                    // an unused direction gets an empty block (0 channels, 0 frames)
                    let input = if info.in_channels == 0 {
                        Block::from_slice_limited(&[], 0, 0, 0, 0)
                    } else {
                        Block::from_slice(input, info.in_channels as u16)
                    };
                    let output = if info.out_channels == 0 {
                        BlockMut::from_slice_limited(&mut [], 0, 0, 0, 0)
                    } else {
                        BlockMut::from_slice(output, info.out_channels as u16)
                    };
                    process_fn(input, output);
                }
            },
        );
        if let Err(e) = result {
            // the handle is dropped here, which closes the stream
            self.status.set_running(false);
            return Err(Error::Backend(Box::new(e)));
        }

        // only kept once it's running, so a failed start doesn't hold the device open
        self.stream_handle = Some(stream_handle);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Error> {
        let _api_call = ApiCall::enter();
        // not running first, so errors while closing aren't reported
        self.status.set_running(false);
        if let Some(mut stream_handle) = self.stream_handle.take() {
            stream_handle.stop();
        }
        Ok(())
    }

    fn status(&self) -> StreamStatus {
        self.status.status()
    }
}

/// Status trackers of all rtaudio hosts. rtaudio has a single error callback per process
/// and doesn't say which stream failed, so stream errors are reported to every running host.
static STATUS_TRACKERS: Mutex<Vec<Weak<StatusTracker>>> = Mutex::new(Vec::new());

thread_local! {
    static IN_API_CALL: Cell<bool> = const { Cell::new(false) };
}

/// Marks the current thread as inside one of our rtaudio calls while it lives.
///
/// rtaudio calls the error callback on the thread that hit the error. Errors raised during
/// our own calls (opening a stream, listing devices, ...) are returned by that call, so they
/// aren't stream failures. Only errors from other threads (the audio threads) are.
struct ApiCall {
    was_in_call: bool,
}

impl ApiCall {
    fn enter() -> Self {
        Self {
            was_in_call: IN_API_CALL.replace(true),
        }
    }
}

impl Drop for ApiCall {
    fn drop(&mut self) {
        IN_API_CALL.set(self.was_in_call);
    }
}

/// Creates a status tracker that receives rtaudio's stream errors.
fn register_status() -> Arc<StatusTracker> {
    static ERROR_CALLBACK: Once = Once::new();
    ERROR_CALLBACK.call_once(|| {
        rtaudio::set_error_callback(report_error);
    });

    let status = Arc::new(StatusTracker::default());
    let mut trackers = STATUS_TRACKERS.lock().unwrap_or_else(|e| e.into_inner());
    // drop the trackers of hosts that are gone, so the list doesn't grow with every new host
    trackers.retain(|tracker| tracker.strong_count() > 0);
    trackers.push(Arc::downgrade(&status));
    status
}

/// rtaudio's error callback: reports stream errors to every running host.
fn report_error(error: RtAudioError) {
    if IN_API_CALL.get() {
        return;
    }
    let stream_error = match error.type_ {
        RtAudioErrorType::Warning => return,
        RtAudioErrorType::DeviceDisconnect => StreamError::DeviceLost,
        _ => StreamError::Backend(error.to_string()),
    };
    let mut trackers = STATUS_TRACKERS.lock().unwrap_or_else(|e| e.into_inner());
    trackers.retain(|tracker| tracker.strong_count() > 0);
    for tracker in trackers.iter().filter_map(Weak::upgrade) {
        // only affects running streams
        tracker.fail(stream_error.clone());
    }
}

/// The rates to suggest after the device opened at `opened` instead of `rejected`.
/// `rejected` was listed but didn't work, so it's removed; the opened rate always works.
fn rates_without(listed: Option<Vec<u32>>, rejected: u32, opened: u32) -> Vec<u32> {
    let mut rates: Vec<u32> = listed
        .unwrap_or_default()
        .into_iter()
        .filter(|&rate| rate != rejected)
        .collect();
    if !rates.contains(&opened) {
        rates.push(opened);
        rates.sort_unstable();
    }
    rates
}

fn to_u16(value: u32) -> u16 {
    value.min(u16::MAX as u32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_rates_drop_the_rejected_rate() {
        assert_eq!(
            rates_without(Some(vec![44100, 48000, 96000]), 96000, 48000),
            vec![44100, 48000]
        );
        // the opened rate is always suggested
        assert_eq!(rates_without(Some(vec![96000]), 96000, 48000), vec![48000]);
        assert_eq!(rates_without(None, 96000, 48000), vec![48000]);
    }

    #[test]
    fn only_errors_from_other_threads_fail_streams() {
        let error = || RtAudioError {
            type_: RtAudioErrorType::SystemError,
            msg: Some("test".into()),
        };
        let status = register_status();
        status.set_running(true);

        // e.g. another host failing to open a stream: returned by that call
        {
            let _api_call = ApiCall::enter();
            report_error(error());
        }
        assert!(status.status().running);

        // a stream's audio thread, even while this thread is in a call
        {
            let _api_call = ApiCall::enter();
            std::thread::spawn(move || report_error(error()))
                .join()
                .unwrap();
        }
        assert!(!status.status().running);
        assert!(matches!(
            status.status().error,
            Some(StreamError::Backend(_))
        ));
    }

    #[test]
    fn dropped_status_trackers_are_removed() {
        for _ in 0..100 {
            drop(register_status());
        }
        let _status = register_status();
        // other tests may register trackers at the same time, so only check it didn't grow
        // with every dropped tracker
        let len = STATUS_TRACKERS.lock().unwrap().len();
        assert!(len < 100, "{len} trackers kept");
    }
}
