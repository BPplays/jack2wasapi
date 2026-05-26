use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceDescription,DeviceNameError};

use log::{info,error,warn};

pub fn description_to_string(
    d: Result<DeviceDescription, DeviceNameError>,
) -> Result<String, DeviceNameError> {
    let desc = d?;
    let mut ext = desc.extended().to_owned();

    if ext.len() <= 1 {
        return Ok(ext.first().cloned().unwrap_or_default());
    }

    ext.sort();
    Ok(ext.join(", "))
}

pub struct AudioBridge {
    is_running: Arc<AtomicBool>,
}

impl AudioBridge {
    pub fn new() -> Self {
        AudioBridge {
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self, wanted_output: &str, buffer_size: u32) -> Result<(), Box<dyn std::error::Error>> {
        // Set running flag to true
        self.is_running.store(true, Ordering::Relaxed);

        // In a full implementation, this would:
        // 1. Connect to JACK server
        // 2. Register JACK input port
        // 3. Enumerate WASAPI devices
        // 4. Open WASAPI output stream
        // 5. Set up ring buffer between the two
        // 6. Implement reconnection logic
        // 7. Handle audio callbacks

        // Placeholder showing what would happen in a proper implementation:
        info!("Audio bridge configuration:");
        info!("  Buffer size: {}", buffer_size);
        info!("  JACK integration: Placeholder (would connect to JACK server)");
        info!("  WASAPI output: {}", wanted_output);
        let host = cpal::default_host();

        let selected = host
            .output_devices()? // this is usually a Result<Devices, _>
            .find_map(|device| {
                let desc = description_to_string(device.description()).ok()?;
                if desc == wanted_output {
                    Some(device)
                } else {
                    None
                }
            });

        match selected {
            Some(device) => {
                println!("matched device: {}",
                    description_to_string(device.description())
                    .unwrap()
                );
                // use `device` here
                }
            None => {
                println!("no matching device found");
                }
        }


        // This is where the actual bridge code would go
        // For now, we just simulate starting it

        Ok(())
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
        info!("Audio bridge stopping...");
    }
}
