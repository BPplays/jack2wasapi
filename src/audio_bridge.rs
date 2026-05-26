use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use thiserror::Error;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, BuildStreamError, DeviceDescription, DevicesError, FromSample, I24, Sample,
    SampleFormat, SizedSample, Stream, StreamConfig, U24,
};

use jack::{AudioIn, AsyncClient, Client, ClientOptions, Control, Port, ProcessScope};

use log::{info, warn};

use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};

use rubato::{
    audioadapter_buffers::direct::SequentialSliceOfVecs, Async, FixedAsync, Resampler,
    SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

#[derive(Debug, Error)]
pub enum DescStringErr {
    #[error("failed to get device description: {0}")]
    DeviceName(#[from] cpal::DeviceNameError),

    #[error("device description was empty")]
    EmptyDescription,
}

#[derive(Debug, Error)]
pub enum AudioBridgeError {
    #[error("unsupported sample format")]
    UnsupportedCPALsampleFormat,

    #[error("can't find output device")]
    OutputDeviceNotFound,

    #[error("DefaultStreamConfigError: {0}")]
    DefaultStreamConfigError(#[from] cpal::DefaultStreamConfigError),

    #[error("PlayStreamError: {0}")]
    PlayStreamError(#[from] cpal::PlayStreamError),

    #[error("BuildStreamError: {0}")]
    BuildStreamError(#[from] cpal::BuildStreamError),

    #[error("jack error: {0}")]
    JackErr(#[from] jack::Error),

    #[error("DescStringErr: {0}")]
    DescStringErr(#[from] DescStringErr),

    #[error("DevicesError: {0}")]
    DevicesError(#[from] DevicesError),
}

pub fn description_to_string(
    d: Result<DeviceDescription, cpal::DeviceNameError>,
) -> Result<String, DescStringErr> {
    let desc = d?;
    let mut ext = desc.extended().to_owned();

    if ext.is_empty() {
        return Err(DescStringErr::EmptyDescription);
    }

    if ext.len() == 1 {
        return Ok(ext[0].clone());
    }

    ext.sort();
    Ok(ext.join(", "))
}

pub struct AudioBridge {
    is_running: Arc<AtomicBool>,
    jack_client: Option<AsyncClient<(), JackProcess>>,
    cpal_stream: Option<Stream>,
    resample_thread: Option<JoinHandle<()>>,
}

impl AudioBridge {
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            jack_client: None,
            cpal_stream: None,
            resample_thread: None,
        }
    }

    pub fn start(&mut self, wanted_output: &str, buffer_size: u32) -> Result<(), AudioBridgeError> {
        if self.is_running.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let host = cpal::default_host();

        let device = host
            .output_devices()?
            .find_map(|device| {
                let desc = description_to_string(device.description()).ok()?;
                if desc == wanted_output {
                    Some(device)
                } else {
                    None
                }
            })
            .ok_or(AudioBridgeError::OutputDeviceNotFound)?;

        info!("matched device: {}", description_to_string(device.description())?);

        let (client, status) = Client::new("jack2wasapi", ClientOptions::default())?;
        info!("connected to JACK: status={status:?}");

        let jack_sr = client.sample_rate() as u32;
        let jack_period = client.buffer_size() as usize;
        info!("jack sample_rate={jack_sr} buffer_size={jack_period}");

        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();

        let mut config: StreamConfig = supported.into();
        config.buffer_size = BufferSize::Fixed(buffer_size);
        config.channels = 1;

        let cpal_sr = config.sample_rate;
        let base_ratio = cpal_sr as f64 / jack_sr as f64;

        let output_channels = config.channels as usize;

        let input_capacity_samples = jack_period.saturating_mul(8).max(1024);
        let output_capacity_samples = jack_period.saturating_mul(16).max(2048);

        info!(
            "base resample ratio={} (cpal_sr={} / jack_sr={})",
            base_ratio, cpal_sr, jack_sr
        );

        let in_rb = HeapRb::<f32>::new(input_capacity_samples);
        let (in_prod, mut in_cons) = in_rb.split();

        let out_rb = HeapRb::<f32>::new(output_capacity_samples);
        let (mut out_prod, out_cons) = out_rb.split();

        let input_overruns = Arc::new(AtomicU64::new(0));
        let output_underruns = Arc::new(AtomicU64::new(0));

        let input_port = client.register_port("input", AudioIn::default())?;
        let jack_process = JackProcess {
            input_port,
            producer: in_prod,
            overruns: input_overruns.clone(),
        };
        let jack_client = client.activate_async((), jack_process)?;
        self.jack_client = Some(jack_client);

        let running = self.is_running.clone();
        let resample_thread = {
            let output_underruns = output_underruns.clone();

            thread::spawn(move || {
                let params = SincInterpolationParameters {
                    sinc_len: 64,
                    f_cutoff: 0.95,
                    interpolation: SincInterpolationType::Cubic,
                    oversampling_factor: 16,
                    window: WindowFunction::BlackmanHarris2,
                };

                let mut resampler = match Async::<f32>::new_sinc(
                    base_ratio,
                    1.055,
                    &params,
                    jack_period,
                    1,
                    FixedAsync::Input,
                ) {
                    Ok(r) => r,
                    Err(err) => {
                        warn!("failed to create rubato resampler: {err}");
                        return;
                    }
                };

                let input_frames = jack_period;
                let output_frames_max = resampler.output_frames_max();

                let mut in_buf = vec![vec![0.0f32; input_frames]; 1];
                let mut out_buf = vec![vec![0.0f32; output_frames_max]; 1];
                let mut current_ratio = base_ratio;

                while running.load(Ordering::Relaxed) {
                    for s in &mut in_buf[0] {
                        match in_cons.try_pop() {
                            Some(v) => *s = v,
                            None => {
                                thread::sleep(Duration::from_micros(500));
                                continue;
                            }
                        }
                    }

                    let input = SequentialSliceOfVecs::new(&in_buf, 1, input_frames)
                        .expect("valid input adapter");
                    let mut output = SequentialSliceOfVecs::new_mut(&mut out_buf, 1, output_frames_max)
                        .expect("valid output adapter");

                    let out_frames_next = resampler.output_frames_next();

                    match resampler.process_into_buffer(&input, &mut output, None) {
                        Ok((_used_in, produced_out)) => {
                            let pushed = out_prod.push_slice(&out_buf[0][..produced_out]);
                            if pushed < produced_out {
                                warn!(
                                    "output ring full — dropped {} samples",
                                    produced_out - pushed
                                );
                            }
                        }
                        Err(err) => {
                            warn!("rubato resample error: {err}");
                            thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                    }

                    let over = input_overruns.swap(0, Ordering::Relaxed);
                    let under = output_underruns.swap(0, Ordering::Relaxed);
                    let net = over as i64 - under as i64;

                    if net != 0 {
                        let drift_ratio = net as f64 / jack_period as f64;
                        let correction = 1.0 + drift_ratio * 0.5;
                        let new_ratio = (base_ratio * correction)
                            .clamp(base_ratio * (
                                1_f64 + (1_f64 - 1.05)
                            ), base_ratio * 1.05);

                        if (new_ratio - current_ratio).abs() > 1e-9 {
                            if let Err(err) = resampler.set_resample_ratio(new_ratio, true) {
                                warn!("failed to retune resampler ratio: {err}");
                            } else {
                                current_ratio = new_ratio;
                            }
                        }
                    }

                    if out_frames_next == 0 {
                        thread::sleep(Duration::from_micros(500));
                    }
                }
            })
        };
        self.resample_thread = Some(resample_thread);

        let err_fn = |err| warn!("CPAL stream error: {err}");

        let stream = match sample_format {
            SampleFormat::F32 => build_output_stream::<f32>(
                &device,
                &config,
                output_channels,
                out_cons,
                output_underruns,
                err_fn,
            )?,
            SampleFormat::I24 => build_output_stream::<I24>(
                &device,
                &config,
                output_channels,
                out_cons,
                output_underruns,
                err_fn,
            )?,
            SampleFormat::U24 => build_output_stream::<U24>(
                &device,
                &config,
                output_channels,
                out_cons,
                output_underruns,
                err_fn,
            )?,
            SampleFormat::I16 => build_output_stream::<i16>(
                &device,
                &config,
                output_channels,
                out_cons,
                output_underruns,
                err_fn,
            )?,
            SampleFormat::U16 => build_output_stream::<u16>(
                &device,
                &config,
                output_channels,
                out_cons,
                output_underruns,
                err_fn,
            )?,
            _other => return Err(AudioBridgeError::UnsupportedCPALsampleFormat),
        };

        stream.play()?;
        self.cpal_stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

        if let Some(handle) = self.resample_thread.take() {
            let _ = handle.join();
        }

        if self.cpal_stream.take().is_some() {
            info!("CPAL stream stopped");
        }

        if let Some(client) = self.jack_client.take() {
            if let Err(err) = client.deactivate() {
                warn!("failed to deactivate JACK client cleanly: {err}");
            }
        }

        info!("Audio bridge stopping...");
    }
}

struct JackProcess {
    input_port: Port<AudioIn>,
    producer: HeapProd<f32>,
    overruns: Arc<AtomicU64>,
}

impl jack::ProcessHandler for JackProcess {
    fn process(&mut self, _client: &Client, ps: &ProcessScope) -> Control {
        let input = self.input_port.as_slice(ps);
        let written = self.producer.push_slice(input);

        if written < input.len() {
            let dropped = input.len() - written;
            self.overruns.fetch_add(dropped as u64, Ordering::Relaxed);
            warn!("input ring buffer full — dropped {} samples", dropped);
        }

        Control::Continue
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    mut consumer: HeapCons<f32>,
    underruns: Arc<AtomicU64>,
    mut err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, BuildStreamError>
where
    T: Sample + SizedSample + FromSample<f32> + Send + 'static,
{
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let sample = match consumer.try_pop() {
                    Some(s) => s,
                    None => {
                        underruns.fetch_add(1, Ordering::Relaxed);
                        0.0
                    }
                };

                let v = T::from_sample(sample);
                for out in frame.iter_mut() {
                    *out = v;
                }
            }
        },
        move |err| err_fn(err),
        None,
    )?;

    Ok(stream)
}
