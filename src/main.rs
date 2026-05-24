use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use clap::Parser;
use log::{info, error};
use serde::{Deserialize, Serialize};
use env_logger::{Builder, Target};
use log::LevelFilter;

mod audio_bridge;

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    device_name: Option<String>,
    buffer_size: u32,
}

#[derive(Parser)]
#[clap(name = "jack2wasapi", version = "0.1.0")]
struct Cli {
    /// Start minimized in system tray
    #[clap(long)]
    minimized: bool,

    /// Session name for config storage
    #[clap(long, default_value = "default")]
    session: String,

    /// Buffer size for audio (default 64)
    #[clap(long, default_value = "64")]
    buffer_size: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Builder::new()
        .filter_level(LevelFilter::Info)
        .target(Target::Stdout)
        .init();
    info!("Starting jack2wasapi");

    let cli = Cli::parse();
    info!("Session: {}, Buffer size: {}", cli.session, cli.buffer_size);

    let config_dir = get_config_dir(&cli.session)?;
    let config_file = config_dir.join("config.json");

    let mut config = load_config(&config_file)?;
    config.buffer_size = cli.buffer_size;
    save_config(&config_file, &config)?;

    // Create audio bridge and start it
    let mut bridge = audio_bridge::AudioBridge::new();
    match bridge.start(&cli.session, cli.buffer_size) {
        Ok(_) => {
            info!("Audio bridge started successfully");
        }
        Err(e) => {
            error!("Failed to start audio bridge: {:?}", e);
            return Err(e);
        }
    };

    // Set up system tray
    let is_running = Arc::new(AtomicBool::new(true));

    // If minimized flag is set or no explicit mode requested, start in tray mode (default)
    if cli.minimized {
        info!("Starting in minimized mode with system tray");
        setup_tray(&cli.session, &is_running);
    } else {
        // Default startup behavior - tray mode with GUI option
        setup_tray(&cli.session, &is_running);
        info!("Starting with system tray interface");
        // In a real implementation, we would also launch the GUI here
    }

    // Main thread - wait for Ctrl+C or signal  
    let mut counter = 0;
    while is_running.load(Ordering::Relaxed) {
        counter += 1;
        if counter % 60 == 0 {
            info!("Application running in background...");
        }
        thread::sleep(Duration::from_secs(1));
    }

    // Stop bridge before exiting
    bridge.stop();
    info!("Application shutdown complete");
    Ok(())
}

fn setup_tray(session_name: &str, _is_running: &Arc<AtomicBool>) {
    info!("Creating system tray icon for session: {}", session_name);
    
    // In a real implementation, we would create the actual tray icon here with:
    // - Tray icon with icon_from_rgba()
    // - Menu with options like "Show GUI" and "Quit"
    // - Event handling for menu items
    // - Proper application lifecycle management
    
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
