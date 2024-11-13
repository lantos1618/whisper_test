extern crate anyhow;
extern crate cpal;
extern crate ringbuf;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::{Consumer, Producer, Split}, HeapRb};

pub fn main() -> Result<(), anyhow::Error> {
    let host = cpal::default_host();

    // Default devices
    let input_device = host.default_input_device().expect("failed to get default input device");
    let output_device = host.default_output_device().expect("failed to get default output device");
    println!("Using default input device: \"{}\"", input_device.name()?);
    println!("Using default output device: \"{}\"", output_device.name()?);

    // Configuration setup
    let config: cpal::StreamConfig = input_device.default_input_config()?.into();

    // Create a HeapRb for our ring buffer
    let buffer_samples = config.sample_rate.0 as usize / 100; // About 10ms of buffer, adjust as needed
    let ring_buffer = HeapRb::<f32>::new(buffer_samples * config.channels as usize);
    let (mut producer, mut consumer) = ring_buffer.split();

    // Input stream callback
    let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        for &sample in data {
            if producer.try_push(sample).is_err() {
                eprintln!("Output stream fell behind: consider increasing buffer size");
            }
        }
    };

    // Output stream callback
    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        for sample in data {
            *sample = match consumer.try_pop() {
                Some(s) => s,
                None => 0.0, // Fill with silence if input lags behind
            };
        }
    };

    // Build streams
    println!("Attempting to build both streams with f32 samples and `{:?}`.", config);
    let input_stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
    let output_stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;
    println!("Successfully built streams.");

    // Start the streams
    input_stream.play()?;
    output_stream.play()?;


    // Wait for the user to press enter
    println!("Press enter to exit...");
    std::io::stdin().read_line(&mut String::new()).unwrap();

    // Prevents the streams from stopping by keeping them alive indefinitely
    std::mem::forget(input_stream);
    std::mem::forget(output_stream);
    Ok(())
}

// Error callback for handling errors in the audio streams
fn err_fn(err: cpal::StreamError) {
    eprintln!("An error occurred on stream: {}", err);
}
