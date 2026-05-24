use dioxus::prelude::*;
use dioxus_desktop::{launch_cfg, Config, WindowBuilder};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

// Application state shared between Dioxus and the audio bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub device_name: Option<String>,
    pub buffer_size: u32,
    pub is_running: bool,
    pub jack_connected: bool,
    pub wasapi_connected: bool,
    pub session_name: String,
    pub error_message: Option<String>,
    pub devices: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            device_name: None,
            buffer_size: 64,
            is_running: false,
            jack_connected: false,
            wasapi_connected: false,
            session_name: "default".to_string(),
            error_message: None,
            devices: vec![],
        }
    }
}

pub fn create_gui_app() -> AppState {
    let initial_state = AppState::default();
    let state = Arc::new(Mutex::new(initial_state));
    
    // Launch the GUI application
    launch_cfg(
        app,
        Config::default().with_window(
            WindowBuilder::new()
                .with_title("jack2wsapi GUI")
                .with_inner_size(800, 600),
        ),
    );
    
    // Return a clone of the state (this would need to be updated for real interactivity)
    state.lock().unwrap().clone()
}

fn app(cx: Scope) -> Element {
    render! {
        div {
            style: {
                "display": "flex",
                "flex-direction": "column",
                "height": "100%",
                "width": "100%",
                "padding": "20px"
            }
            h1 { "jack2wsapi Configuration" }
            
            div {
                style: {
                    "display": "flex",
                    "flex-direction": "column",
                    "gap": "10px",
                    "margin-bottom": "20px"
                }
                p { "JACK Connected: false" }
                p { "WASAPI Connected: false" }
                p { "Running: false" }
            }
            
            div {
                style: {
                    "display": "flex",
                    "flex-direction": "column",
                    "gap": "10px",
                    "margin-bottom": "20px"
                }
                
                h2 { "Audio Device Configuration" }
                
                label { "Buffer Size: 64" }
                input { 
                    r#type: "range", 
                    min: "16", 
                    max: "1024", 
                    step: "16",
                    value: "64"
                }
                
                label { "Device: None" }
                select {
                    option { value: "default", "Default Device" }
                }
            }
            
            div {
                style: {
                    "display": "flex",
                    "gap": "10px",
                    "margin-top": "20px"
                }
                button { "Start" }
                button { "Stop" }
                button { "Apply" }
            }
        }
    }
}