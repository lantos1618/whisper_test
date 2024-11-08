use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use textplots::{Chart, Plot, Shape};
use std::sync::{Arc, Mutex};

fn main() {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("Failed to get input device");
    let config = device.default_input_config().expect("Failed to get default config");

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    // Shared state for the maximum decibel value
    let max_db_value = Arc::new(Mutex::new(f32::NEG_INFINITY));

    let stream = device.build_input_stream(
        &config.clone().into(),
        {
            let max_db_value = Arc::clone(&max_db_value);
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Convert the input data to complex numbers
                let mut buffer: Vec<Complex<f32>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();

                // Create an FFT planner and perform the FFT
                let mut planner = FftPlanner::new();
                let fft = planner.plan_fft_forward(buffer.len());
                fft.process(&mut buffer);

                // Calculate magnitudes and frequencies
                let magnitudes: Vec<f32> = buffer.iter().map(|c| c.norm()).collect();
                let sample_rate = config.sample_rate().0 as f32;
                let frequencies: Vec<f32> = (0..buffer.len()).map(|i| i as f32 * sample_rate / buffer.len() as f32).collect();

                // Convert magnitudes to decibels and normalize
                let magnitudes_db: Vec<f32> = magnitudes.iter().map(|&m| 20.0 * m.log10()).collect();
                let max_db = magnitudes_db.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let min_db = magnitudes_db.iter().cloned().fold(f32::INFINITY, f32::min);
                let normalized_db: Vec<f32> = magnitudes_db.iter().map(|&m| (m - min_db) / (max_db - min_db) * 100.0).collect();

                // Apply a noise gate: set a threshold below which magnitudes are considered noise
                let noise_threshold = 20.0; // Adjust this threshold as needed
                let gated_db: Vec<f32> = normalized_db.iter().map(|&m| if m < noise_threshold { 0.0 } else { m }).collect();

                // Update the maximum decibel value
                {
                    let mut max_db_lock = max_db_value.lock().unwrap();
                    if max_db > *max_db_lock {
                        *max_db_lock = max_db;
                    }
                }

                // Plot the frequency spectrum for the human voice range
                println!("Frequency Spectrum (Human Voice Range):");
                Chart::new(180, 40, 85.0, 3400.0) // Adjusted range for human voice
                    .lineplot(&Shape::Continuous(Box::new(move |x| {
                        // Interpolate the gated magnitude for the given frequency x
                        let mut closest = (0.0, 0.0);
                        for (&f, &m_gated) in frequencies.iter().zip(gated_db.iter()) {
                            if (f - x).abs() < (closest.0 - x).abs() {
                                closest = (f, m_gated);
                            }
                        }
                        closest.1
                    })))
                    .display();
            }
        },
        err_fn,
        None,
    ).expect("Failed to build input stream");

    stream.play().expect("Failed to play stream");

    std::thread::sleep(std::time::Duration::from_secs(10));

    // Access the maximum decibel value outside the loop
    let max_db_value = *max_db_value.lock().unwrap();
    println!("Maximum Decibel Value: {:.2} dB", max_db_value);
}