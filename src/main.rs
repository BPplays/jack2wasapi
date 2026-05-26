use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use cpal::traits::{DeviceTrait, HostTrait};

use clap::Parser;
use log::{info, error};
use serde::{Deserialize, Serialize};
use env_logger::{Builder, Target};
use log::LevelFilter;

mod audio_bridge;
mod gui;

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    device_name: Option<String>,
    buffer_size: u32,
}

#[derive(Parser)]
#[clap(name = "jack2wasapi", version = "0.1.0")]
struct Cli {
    /// Session name for config storage
    #[clap(long, default_value = "")]
    output: String,

    /// Buffer size for audio (default 64)
    #[clap(long, default_value = "64")]
    buffer_size: u32,

    #[arg(long)]
    list: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Builder::new()
        .filter_level(LevelFilter::Info)
        .target(Target::Stdout)
        .init();
    info!("Starting jack2wasapi");

    let cli = Cli::parse();

    if cli.list {
        println!("Listing items...");
        let host = cpal::default_host();

        println!("=== INPUT ===");
        for d in host.input_devices()? {
            println!(
                "IN  : {} |",
                audio_bridge::description_to_string(d
                    .description())
                    .unwrap_or("unknown"
                    .to_string()),
            );
        }

        println!("\n=== OUTPUT ===");
        for d in host.output_devices()? {
            println!(
                "OUT : {} |",
                audio_bridge::description_to_string(d
                    .description())
                    .unwrap_or("unknown"
                    .to_string()),
            );
        }
        return Ok(());
    }
    info!("Output: {}, Buffer size: {}", cli.output, cli.buffer_size);

    // Create audio bridge and start it
    let mut bridge = audio_bridge::AudioBridge::new();
    match bridge.start(&cli.output, cli.buffer_size) {
        Ok(_) => {
            info!("Audio bridge started successfully");
        }
        Err(e) => {
            error!("Failed to start audio bridge: {:?}", e);
            return Err(e.into());
        }
    };

    // Set up system tray
    let is_running = Arc::new(AtomicBool::new(true));

    // // If minimized flag is set or no explicit mode requested, start in tray mode (default)
    // if cli.minimized {
    //     info!("Starting in minimized mode with system tray");
    //     gui::run(true);
    // } else {
    //     gui::run(false);
    //     info!("starting gui");
    // }

    // Main thread - wait for Ctrl+C or signal
    let mut counter = 0;
    while is_running.load(Ordering::Relaxed) {
        counter += 1;
        if counter % 60 == 0 {
            info!("Application running in background...");
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Stop bridge before exiting
    bridge.stop();
    info!("Application shutdown complete");
    Ok(())
}

fn setup_tray(session_name: &str, _is_running: &Arc<AtomicBool>) {
    info!("Creating system tray icon for session: {}", session_name);

    // This would be implemented with the actual tray-icon crate functionality
    // For now we just log that it's ready
    info!("System tray is ready with basic functionality");
}

fn get_config_dir(session: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .ok_or("Could not determine config directory")?;
    let config_dir = config_dir.join("jack2wasapi").join(session);
    std::fs::create_dir_all(&config_dir)?;
    Ok(config_dir)
}

fn load_config(config_file: &std::path::Path) -> Result<Config, Box<dyn std::error::Error>> {
    if config_file.exists() {
        let contents = std::fs::read_to_string(config_file)?;
        let config: Config = serde_json::from_str(&contents)?;
        Ok(config)
    } else {
        Ok(Config {
            device_name: None,
            buffer_size: 64,
        })
    }
}

fn save_config(config_file: &std::path::Path, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let contents = serde_json::to_vec_pretty(config)?;
    std::fs::write(config_file, contents)?;
    Ok(())
}
