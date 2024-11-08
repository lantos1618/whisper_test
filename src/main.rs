use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use textplots::{Chart, Plot, Shape};

fn main() {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("Failed to get input device");
    let config = device.default_input_config().expect("Failed to get default config");

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_input_stream(
        &config.clone().into(),
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

            // Find the top 100 frequencies
            let mut freq_magnitudes: Vec<(f32, f32)> = frequencies.iter().zip(magnitudes.iter()).map(|(&f, &m)| (f, m)).collect();
            freq_magnitudes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let top_100: Vec<(f32, f32)> = freq_magnitudes.into_iter().take(100).collect();

            // Scale the magnitudes for better visualization
            let max_magnitude = top_100.iter().map(|&(_, m)| m).fold(0.0, f32::max);
            let scaled_top_100: Vec<(f32, f32)> = top_100.iter().map(|&(f, m)| (f, m / max_magnitude)).collect();

            // Plot the top 100 frequencies using a continuous line plot
            println!("Top 100 Frequencies:");
            Chart::new(120, 30, 0.0, 24000.0)
                .lineplot(&Shape::Continuous(Box::new(move |x| {
                    // Interpolate the magnitude for the given frequency x
                    let mut closest = (0.0, 0.0);
                    for &(f, m) in &scaled_top_100 {
                        if (f - x).abs() < (closest.0 - x).abs() {
                            closest = (f, m);
                        }
                    }
                    closest.1
                })))
                .display();
        },
        err_fn,
        None,
    ).expect("Failed to build input stream");

    stream.play().expect("Failed to play stream");

    std::thread::sleep(std::time::Duration::from_secs(10));
}