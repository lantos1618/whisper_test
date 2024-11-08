use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use textplots::{Chart, Plot, Shape};
use std::sync::{Arc, Mutex};
use termion::{clear, cursor};
use termion::terminal_size;
use std::sync::mpsc;
use std::convert::TryInto;

use serde::Deserialize;
use std::{fs, thread, time::Duration, time::SystemTime};
use std::io::{stdout, Write};
use crossterm::{execute, terminal::{Clear, ClearType}};
use std::path::Path;

fn main() {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("Failed to get input device");
    let config = device.default_input_config().expect("Failed to get default config");

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    // Shared state for the maximum decibel value
    let max_db_value = Arc::new(Mutex::new(f32::NEG_INFINITY));

    // Channel for sending data to the plotting thread
    let (tx, rx) = mpsc::channel();

    let stream = device.build_input_stream(
        &config.clone().into(),
        {
            let max_db_value = Arc::clone(&max_db_value);
            let tx = tx.clone();
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Increase the FFT size by zero-padding the input data
                let fft_size = 4096;
                let mut buffer: Vec<Complex<f32>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();
                buffer.resize(fft_size, Complex::new(0.0, 0.0)); // Zero-padding

                // Create an FFT planner and perform the FFT
                let mut planner = FftPlanner::new();
                let fft = planner.plan_fft_forward(buffer.len());
                fft.process(&mut buffer);

                // Calculate magnitudes and frequencies
                let magnitudes: Vec<f32> = buffer.iter().map(|c| c.norm()).collect();
                let sample_rate = config.sample_rate().0 as f32;
                let frequencies: Vec<f32> = (0..buffer.len()).map(|i| i as f32 * sample_rate / buffer.len() as f32).collect();

                // Update maximum decibel value to track highest signal
                {
                    let mut max_db_lock = max_db_value.lock().unwrap();
                    let max_magnitude = magnitudes.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let max_db = 20.0 * max_magnitude.log10();
                    if max_db > *max_db_lock {
                        *max_db_lock = max_db;
                    }
                }

                // Send data to the plotting thread
                tx.send((frequencies, magnitudes)).expect("Failed to send data to plotting thread");
            }
        },
        err_fn,
        None,
    ).expect("Failed to build input stream");

    stream.play().expect("Failed to play stream");

    // Load ASCII GIF
    let file_path = "ascii_gif_frames.json";
    let mut last_modified_time: Option<SystemTime> = None;
    let mut ascii_gif = load_ascii_gif(file_path).expect("Failed to load ASCII GIF");
    let frame_delay = Duration::from_millis(ascii_gif.frame_duration);

    // Plotting and ASCII GIF display loop
    thread::spawn(move || {
        let mut frame_index = 0;
        loop {
            // Check if file has been modified
            if let Ok(metadata) = fs::metadata(file_path) {
                let modified_time = metadata.modified().expect("Failed to get modified time");

                // Reload the JSON file if it has changed
                if Some(modified_time) != last_modified_time {
                    println!("File has changed, reloading...");
                    ascii_gif = load_ascii_gif(file_path).expect("Failed to reload ASCII GIF");
                    last_modified_time = Some(modified_time);
                }
            }

            // Try to receive data from the audio processing thread without blocking
            if let Ok((frequencies, magnitudes)) = rx.try_recv() {
                // Clear the terminal and get its size
                print!("{}{}", clear::All, cursor::Goto(1, 1));
                let (width, height) = terminal_size().unwrap_or((180, 40));

                // Calculate 90% of the terminal size
                let width_90: u16 = ((width as f32 * 0.9) as u16).try_into().unwrap();
                let height_90: u16 = ((height as f32 * 0.9) as u16).try_into().unwrap();

                // Calculate padding to center the chart
                let horizontal_padding = (width as u16 - width_90) / 2;
                let vertical_padding = (height as u16 - height_90) / 2;

                // Add padding
                let plot_width = width_90 * 2;
                let plot_height = height_90 * 2;

                // Move cursor to start position with padding
                print!("{}", cursor::Goto(horizontal_padding, vertical_padding));

                // Plotting for debugging: Only within human voice range
                println!("Frequency Spectrum (Human Voice Range):");
                Chart::new(plot_width.into(), plot_height.into(), 0.0, 2400.0)
                    .lineplot(&Shape::Continuous(Box::new(|x| {
                        // Interpolate the normalized magnitude for the given frequency x
                        let mut closest = (0.0, 0.0);
                        for (&f, &m_norm) in frequencies.iter().zip(magnitudes.iter()) {
                            if (f - x).abs() < (closest.0 - x).abs() {
                                closest = (f, m_norm);
                            }
                        }
                        closest.1
                    })))
                    .display();
            }

            // Display the current ASCII frame
            if let Some(frame) = ascii_gif.frames.get(frame_index) {
                println!("{}", frame);
            }

            // Increment frame index and wrap around if necessary
            frame_index = (frame_index + 1) % ascii_gif.frames.len();

            // Sleep to match the frame rate of the ASCII GIF
            thread::sleep(frame_delay);
        }
    });

    // Run indefinitely
    thread::sleep(std::time::Duration::from_secs(100));
}



#[derive(Deserialize)]
struct AsciiGif {
    frame_duration: u64,  // Frame duration in milliseconds
    frames: Vec<String>,
}

// Function to load JSON file and return AsciiGif struct
fn load_ascii_gif(path: &str) -> Result<AsciiGif, Box<dyn std::error::Error>> {
    let data = fs::read_to_string(path)?;
    let ascii_gif: AsciiGif = serde_json::from_str(&data)?;
    Ok(ascii_gif)
}
