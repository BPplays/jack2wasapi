use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, BuildStreamError, DeviceDescription, DevicesError, FromSample, I24, Sample, SampleFormat, SizedSample, Stream, StreamConfig, U24
};

use jack::{AudioIn, AsyncClient, Client, ClientOptions, Control, Port, ProcessScope};

use log::{info, warn};

use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};

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
}

impl AudioBridge {
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            jack_client: None,
            cpal_stream: None,
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
                if desc == wanted_output { Some(device) } else { None }
            })
        .ok_or(AudioBridgeError::OutputDeviceNotFound)?;

        info!("matched device: {}", description_to_string(device.description())?);

        // Connect to JACK *first* so we know the real sample rate.  // <-- FIX (moved up)
        let (client, status) = Client::new("jack2wasapi", ClientOptions::default())?;
        info!("connected to JACK: status={status:?}");

        let jack_sr   = client.sample_rate() as u32;
        let jack_period = client.buffer_size() as usize;   // <-- FIX: real JACK period
        info!("jack sample_rate={jack_sr} buffer_size={jack_period}");

        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();

        let mut config: StreamConfig = supported.into();
        config.sample_rate = jack_sr;   // <-- FIX: match JACK rate
        config.buffer_size = BufferSize::Fixed(buffer_size);
        config.channels = 1;

        let channels = config.channels as usize;

        // Size the ring off the *JACK* period (the producer side), not the CPAL buffer.
        // 8x gives ~170 ms headroom at 48 kHz / 1024 period — plenty for scheduling jitter.  // <-- FIX
        let ring_capacity_samples = jack_period
            .saturating_mul(channels)
            .saturating_mul(8);

        info!("rb samples: {}", ring_capacity_samples);
        let rb = HeapRb::<f32>::new(ring_capacity_samples);
        let (producer, consumer) = rb.split();

        let input_port = client.register_port("input", AudioIn::default())?;
        let jack_process = JackProcess { input_port, producer };
        let jack_client = client.activate_async((), jack_process)?;
        self.jack_client = Some(jack_client);

        let err_fn = |err| warn!("CPAL stream error: {err}");

        let stream = match sample_format {
            SampleFormat::F32 => build_output_stream::<f32>(&device, &config, channels, consumer, err_fn)?,
            SampleFormat::I24 => build_output_stream::<I24>(&device, &config, channels, consumer, err_fn)?,
            SampleFormat::U24 => build_output_stream::<U24>(&device, &config, channels, consumer, err_fn)?,
            SampleFormat::I16 => build_output_stream::<i16>(&device, &config, channels, consumer, err_fn)?,
            SampleFormat::U16 => build_output_stream::<u16>(&device, &config, channels, consumer, err_fn)?,
            _other => return Err(AudioBridgeError::UnsupportedCPALsampleFormat),
        };

        stream.play()?;
        self.cpal_stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

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
}

impl jack::ProcessHandler for JackProcess {
    fn process(&mut self, _client: &Client, ps: &ProcessScope) -> Control {
        let input = self.input_port.as_slice(ps);
        // info!("input is len: {}", input.len());
        let written = self.producer.push_slice(input);
        if written < input.len() {                          // <-- FIX: warn on overrun
            warn!("ring buffer full — dropped {} samples", input.len() - written);
        }
        Control::Continue
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    mut consumer: HeapCons<f32>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, BuildStreamError>
where
    T: Sample + SizedSample + FromSample<f32> + Send + 'static,
{
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
            // Fill per frame, not per raw sample.
            // This avoids left/right interleaving crackle when the JACK side is mono.
            for frame in data.chunks_mut(channels) {
                let sample = consumer.try_pop().unwrap_or(0.0);
                let v = T::from_sample(sample);

                for out in frame.iter_mut() {
                    *out = v;
                }
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
