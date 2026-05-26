use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, BuildStreamError, I24, Sample, SampleFormat, SizedSample, Stream, StreamConfig, U24};
use jack::{AudioIn, AsyncClient, Client, ClientOptions, Control, Port, ProcessScope};
use log::{info, warn};
use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};
use cpal::{FromSample};

#[derive(Debug, Error)]
pub enum DescStringErr {
    #[error("failed to get device description: {0}")]
    DeviceName(#[from] cpal::DeviceNameError),

    #[error("device description was empty")]
    EmptyDescription,
}

#[derive(Debug, Error)]
pub enum AudioBridgeError {
    #[error("device description was empty")]
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
}

pub fn description_to_string(
    d: Result<cpal::DeviceDescription, cpal::DeviceNameError>,
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

    pub fn start(
        &mut self,
        wanted_output: &str,
        buffer_size: u32,
    ) -> Result<(), AudioBridgeError> {
        if self.is_running.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let host = cpal::default_host();

        let device = host
            .output_devices().unwrap()
            .find_map(|device| {
                let desc = description_to_string(device.description()).ok()?;
                if desc == wanted_output {
                    Some(device)
                } else {
                    None
                }
            }).ok_or(AudioBridgeError::OutputDeviceNotFound).unwrap();

        info!(
            "matched device: {}",
            description_to_string(device.description())?
        );

        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();

        let mut config: StreamConfig = supported.into();
        config.buffer_size = BufferSize::Fixed(buffer_size);

        let channels = config.channels as usize;
        let ring_capacity_samples = (buffer_size as usize)
            .saturating_mul(channels)
            .saturating_mul(8);

        let rb = HeapRb::<f32>::new(ring_capacity_samples);
        let (producer, consumer) = rb.split();

        let (client, status) = Client::new("jack2wasapi", ClientOptions::default())?;
        info!("connected to JACK: status={status:?}");
        info!(
            "jack sample_rate={} buffer_size={}",
            client.sample_rate(),
            client.buffer_size()
        );

        let input_port = client.register_port("input", AudioIn::default())?;

        let jack_process = JackProcess {
            input_port,
            producer,
        };

        let jack_client = client.activate_async((), jack_process)?;
        self.jack_client = Some(jack_client);

        let err_fn = |err| warn!("CPAL stream error: {err}");

        let stream = match sample_format {
            SampleFormat::F32 => build_output_stream::<f32>(&device, &config, consumer, err_fn)?,
            SampleFormat::I24 => build_output_stream::<I24>(&device, &config, consumer, err_fn)?,
            SampleFormat::U24 => build_output_stream::<U24>(&device, &config, consumer, err_fn)?,
            SampleFormat::I16 => build_output_stream::<i16>(&device, &config, consumer, err_fn)?,
            SampleFormat::U16 => build_output_stream::<u16>(&device, &config, consumer, err_fn)?,
            other => {
                return Err(AudioBridgeError::UnsupportedCPALsampleFormat);
            }
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

        for &sample in input {
            if self.producer.try_push(sample).is_err() {
                break;
            }
        }

        Control::Continue
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut consumer: HeapCons<f32>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, BuildStreamError>
where
    T: Sample + SizedSample + FromSample<f32> + Send + 'static,
{
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
            for out in data.iter_mut() {
                let sample = consumer.try_pop().unwrap_or(0.0);
                *out = T::from_sample(sample);
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
