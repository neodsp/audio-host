use std::{
    fmt::Debug,
    sync::{Arc, Mutex, Once, Weak},
};

use rtaudio::{DeviceParams, Host, RtAudioErrorType, StreamConfig, StreamFlags, StreamHandle};

use crate::{
    AudioBackend, Block, BlockMut, Config, DeviceInfo, Error, StatusTracker, StreamError,
    StreamStatus, find_by_name,
};

pub struct AudioHost {
    api: rtaudio::Api,
    input_device: Option<rtaudio::DeviceInfo>,
    output_device: Option<rtaudio::DeviceInfo>,
    stream_handle: Option<StreamHandle>,
    status: Arc<StatusTracker>,
}

impl Debug for AudioHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioHost")
            .field("backend", &"RtAudio")
            .field("is_running", &self.stream_handle.is_some())
            .field("apis", &self.apis())
            .field("inputs", &self.inputs())
            .field("outputs", &self.outputs())
            .finish()
    }
}

impl AudioBackend for AudioHost {
    fn new() -> Result<Self, Error> {
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
        self.api = find_by_name(
            rtaudio::compiled_apis().iter().copied(),
            |api| Some(api.get_display_name()),
            name,
        )
        .ok_or(Error::NotFound)?;

        let host = Host::new(self.api).map_err(|e| Error::Backend(Box::new(e)))?;
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

        let stream_handle = Host::new(self.api)
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
                available: available_rates.unwrap_or_else(|| vec![info.sample_rate]),
            });
        }

        let status = self.status.clone();
        self.status.set_running(true);
        self.stream_handle = Some(stream_handle);
        self.stream_handle
            .as_mut()
            .map(|handle| {
                handle.start(
                    move |buffers: rtaudio::Buffers<'_>,
                          info: &rtaudio::StreamInfo,
                          stream_status: rtaudio::StreamStatus| {
                        // input overflow or output underflow since the last callback
                        if !stream_status.is_empty() {
                            status.add_xrun();
                        }
                        if let rtaudio::Buffers::Float32 { output, input } = buffers {
                            let input = Block::from_slice(input, info.in_channels.max(1) as u16);
                            let output =
                                BlockMut::from_slice(output, info.out_channels.max(1) as u16);
                            process_fn(input, output);
                        }
                    },
                )
            })
            .transpose()
            .map_err(|e| {
                self.status.set_running(false);
                Error::Backend(Box::new(e))
            })?;

        Ok(())
    }

    fn stop(&mut self) -> Result<(), Error> {
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
/// and doesn't say which stream failed, so errors are reported to every running host.
static STATUS_TRACKERS: Mutex<Vec<Weak<StatusTracker>>> = Mutex::new(Vec::new());

/// Creates a status tracker that receives rtaudio's stream errors.
fn register_status() -> Arc<StatusTracker> {
    static ERROR_CALLBACK: Once = Once::new();
    ERROR_CALLBACK.call_once(|| {
        rtaudio::set_error_callback(|error| {
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
        });
    });

    let status = Arc::new(StatusTracker::default());
    STATUS_TRACKERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::downgrade(&status));
    status
}

fn to_u16(value: u32) -> u16 {
    value.min(u16::MAX as u32) as u16
}
