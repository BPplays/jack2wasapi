# JACK to WASAPI Audio Bridge Implementation Guide

## Overview 
This document outlines the implementation approach for creating a bridge between JACK input streams and WASAPI output streams. The goal is to allow JACK clients to route audio data to Windows WASAPI output devices.

## Architecture Requirements

### Core Components 
1. **JACK Client** - For connecting to and reading from JACK server
2. **WASAPI Output Stream** - For playing audio to Windows audio devices  
3. **Ring Buffer** - For asynchronous data transfer between JACK and WASAPI
4. **Threading Model** - Proper coordination between audio callbacks and main thread

### Key Integration Points
- JACK input ports → Ring buffer (reader)
- Ring buffer (writer) → WASAPI output stream callback
- Synchronized buffer management between async callbacks
- Error handling and stream reconnection

## JACK Integration (using `jack` crate)

### Client Setup
```rust
let (client, _status) = jack::Client::new(
    "jack2wasapi_bridge", 
    jack::ClientOptions::default()
).unwrap();
```

### Input Port Registration
```rust
let input_port_l: jack::Port<jack::AudioIn> = client
    .register_port("input_l", jack::AudioIn::default())
    .unwrap();

let input_port_r: jack::Port<jack::AudioIn> = client
    .register_port("input_r", jack::AudioIn::default())
    .unwrap();
```

### Processing Callback
```rust
let process = jack::contrib::ClosureProcessHandler::new(
    move |_: &jack::Client, ps: &jack::ProcessScope| -> jack::Control {
        let input_l_slice = input_port_l.as_slice(ps);
        let input_r_slice = input_port_r.as_slice(ps);
        
        // Write to ring buffer - this part will be handled by the bridge
        // For now we just access the slices
        jack::Control::Continue
    },
);
```

## WASAPI Integration (using `cpal` crate)

### Device Enumeration
```rust
let host = cpal::default_host();
let output_devices: Vec<_> = host.output_devices().unwrap().collect();
let default_output = host.default_output_device().unwrap();
```

### Stream Configuration
```rust
let config = default_output
    .default_output_config()
    .unwrap()
    .into();
```

### Output Stream Callback
```rust
let stream = device.build_output_stream(
    &config,
    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        // Read from ring buffer and fill the output buffer
        // This will be our bridge implementation
    },
    move |err| {
        eprintln!("WASAPI stream error: {:?}", err);
    },
    None
);
```

## Ring Buffer Implementation

The bridge uses a lock-free ring buffer to synchronize data between the JACK input callback and WASAPI output callback.

### Buffer Management Approach 
1. Create a ring buffer with size = buffer_size * 2 (to avoid under/overruns) 
2. **JACK Input Callback**:
   - Read audio data from input ports 
   - Write to ring buffer (blocking if full)
3. **WASAPI Output Callback**:  
   - Read audio data from ring buffer (blocking if empty)
   - Write to output stream buffer

### Buffer Size Considerations
- JACK typically operates with smaller buffers (32-128 frames)
- WASAPI can work with larger buffers (1024+ frames)  
- The buffer size in the ring buffer should be chosen to minimize latency while allowing for proper buffering

## Threading and Synchronization

### Multi-threading Approach
Since JACK and WASAPI operate on different threads, proper synchronization is crucial:

1. **Main Thread**: 
   - Manages JACK client
   - Initializes and manages WASAPI streams
   - Handles application lifecycle (start/stop)

2. **JACK Callback Thread**:
   - Audio input callback for JACK

3. **WASAPI Callback Thread**: 
   - Audio output callback for WASAPI

### Synchronization Strategy
- Use a thread-safe ring buffer like `ringbuf` crate
- Ensure consistent frame sizes and sample formats between interfaces
- Handle buffer underruns/overruns gracefully
- Implement proper error handling for both JACK and WASAPI

## Specific API Calls and Implementation

### JACK API Calls
1. `jack::Client::new()` - Creates JACK client
2. `client.register_port()` - Registers input/output ports  
3. `client.activate_async()` - Activates client with processing callback
4. `Port::as_slice()` - Gets reference to input slice
5. `Port::as_mut_slice()` - Gets reference to output slice

### WASAPI API Calls (via CPAL)
1. `cpal::default_host()` - Gets default audio host
2. `host.output_devices()` - Enumerates output devices  
3. `device.default_output_config()` - Gets default output configuration
4. `device.build_output_stream()` - Creates audio output stream
5. `stream.play()` - Starts audio stream

## Implementation Strategy

1. **Initialize JACK client** and set up input ports
2. **Enumerate WASAPI devices** and select appropriate output device
3. **Configure stream settings** for WASAPI (sample rate, channels, format)
4. **Create ring buffer** large enough to handle both JACK and WASAPI buffer sizes
5. **Set up JACK processing callback** to read from ports and write to ring buffer 
6. **Set up WASAPI output callback** to read from ring buffer and write to output buffer
7. **Implement proper error handling** and graceful shutdown

## Key Challenges and Solutions

### 1. Buffer Size Mismatch
**Problem**: JACK typically uses small buffers (32-128 frames) while WASAPI supports larger buffers (1024+ frames)
**Solution**: Use a sufficiently large ring buffer and convert buffer sizes appropriately

### 2. Synchronization and Latency
**Problem**: Audio callbacks are async and must maintain real-time performance
**Solution**: Use lock-free ring buffers, avoid blocking in callbacks, use proper thread safety

### 3. Error Handling
**Problem**: Both JACK and WASAPI can fail in various ways  
**Solution**: Comprehensive error handling with recovery strategies

### 4. Data Format Matching
**Problem**: Different systems may have different sample formats or channel layouts
**Solution**: Ensure consistent 32-bit float format, handle mono/stereo conversion properly

## Recommended Implementation Pattern

```rust
// In audio_bridge.rs

use ringbuf::RingBuffer;
use std::sync::{Arc, Mutex};

pub struct AudioBridge {
    jack_client: Option<jack::Client>,
    wasapi_stream: Option<cpal::Stream>,
    ring_buffer: Arc<RingBuffer<f32>>,
    is_running: Arc<AtomicBool>,
}

impl AudioBridge {
    pub fn new() -> Self {
        // Initialize with default values
        let buffer_size = 1024; // or configurable
        let ring_buffer = RingBuffer::<f32>::new(buffer_size * 2); // Double size to avoid issues
        let ring_buffer = Arc::new(ring_buffer);
        
        AudioBridge {
            jack_client: None,
            wasapi_stream: None,
            ring_buffer,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }
    
    pub fn start(&mut self, session_name: &str, buffer_size: u32) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Connect to JACK
        let (client, _status) = jack::Client::new("jack2wasapi_bridge", jack::ClientOptions::default())?;
        
        // 2. Register JACK input ports
        let in_port_l = client.register_port("input_l", jack::AudioIn::default())?;
        let in_port_r = client.register_port("input_r", jack::AudioIn::default())?;
        
        // 3. Enumerate WASAPI devices
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        
        // 4. Configure WASAPI stream
        let config = device.default_output_config()?.into();
        let ring_buffer_clone = self.ring_buffer.clone();
        
        // 5. Create WASAPI output stream with callback that reads from ring buffer
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Read from ring buffer and write to output data
                // Implementation detail - would read from the shared ring buffer  
            },
            move |err| {
                eprintln!("WASAPI error: {:?}", err);
            },
            None
        )?;
        
        // 6. Set up JACK processing callback that writes to ring buffer
        let process = jack::contrib::ClosureProcessHandler::new(
            move |_: &jack::Client, ps: &jack::ProcessScope| -> jack::Control {
                let in_l = in_port_l.as_slice(ps);
                let in_r = in_port_r.as_slice(ps);
                
                // Write to ring buffer - this is where it needs to be implemented
                // Implementation detail: push data from JACK slices into ring buffer
                
                jack::Control::Continue
            },
        );
        
        let _active_client = client.activate_async((), process)?;
        
        // Store handles
        self.jack_client = Some(client);
        self.wasapi_stream = Some(stream);
        self.is_running.store(true, Ordering::Relaxed);
        
        Ok(())
    }
}
```

## Testing Approach

1. **JACK Server Setup**: Start JACK server first (using qjackctl or jackd)
2. **Audio Graph Setup**: Connect input ports in JACK to establish routing
3. **WASAPI Device**: Select appropriate Windows output audio device
4. **Verification**: Monitor both audio streams to ensure proper data flow

This approach should provide a robust, low-latency bridge between the JACK audio system and Windows WASAPI output streams with proper buffering, synchronization, and error handling.