extern crate anyhow;
extern crate cpal;
extern crate ringbuf;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapRb,
};

#[cfg(test)]
mod tests {
    use kanal::bounded;
    use super::*;

    /// Utility function for setting up host, devices, and stream configuration for tests.
    fn setup_host() -> Result<(cpal::Device, cpal::Device, cpal::StreamConfig), anyhow::Error> {
        let host = cpal::default_host();

        // Get default input and output devices
        let input_device = host.default_input_device().expect("Failed to get input device");
        let output_device = host.default_output_device().expect("Failed to get output device");

        // Configure the stream
        let config: cpal::StreamConfig = input_device.default_input_config()?.into();

        println!("Using input device: {}", input_device.name()?);
        println!("Using output device: {}", output_device.name()?);

        Ok((input_device, output_device, config))
    }

    #[test]
    fn test_kanal_f32() -> Result<(), anyhow::Error> {
        let (input_device, output_device, config) = setup_host()?;

        let sample_rate = config.sample_rate.0 as usize;
        let channels = config.channels as usize;
        let buffer_samples = sample_rate / 100 * channels;
        let (tx, rx) = bounded(buffer_samples);

        // Input stream callback for `f32` samples
        let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                if tx.try_send(sample).is_err() {
                    eprintln!("Warning: output stream fell behind; sample dropped.");
                }
            }
        };

        // Output stream callback for `f32` samples
        let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in output {
                *sample = match rx.try_recv() {
                    Ok(val) => {
                        match val {
                            Some(val) => val,
                            None => 0.0,
                        }
                    },
                    Err(_) => 0.0
                };
            }
        };

        // Build and run streams
        let input_stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
        let output_stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;

        input_stream.play()?;
        output_stream.play()?;

        println!("Streaming `f32`. Press enter to exit...");
        std::io::stdin().read_line(&mut String::new()).unwrap();

        Ok(())
    }

    #[test]
    fn test_kanal_f32_slice() -> Result<(), anyhow::Error> {
        let (input_device, output_device, config) = setup_host()?;

        let sample_rate = config.sample_rate.0 as usize;
        let channels = config.channels as usize;
        let buffer_samples = sample_rate / 100 * channels;
        let (tx, rx) = bounded(buffer_samples);

        // Input stream callback for `[f32]` slices
        let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if tx.try_send(data.to_vec()).is_err() {
                eprintln!("Warning: output stream fell behind; slice dropped.");
            }
        };

        // Output stream callback for `[f32]` slices
        let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            match rx.try_recv() {
                Ok(val) => {
                   match val {
                        Some(val) => {
                            for (i, sample) in output.iter_mut().enumerate().take(val.len()) {
                                *sample = val[i];
                            }
                        },
                        None => (),
                   }
                },
                Err(_) => (),
            }
        };

        // Build and run streams
        let input_stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
        let output_stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;

        input_stream.play()?;
        output_stream.play()?;

        println!("Streaming `[f32]`. Press enter to exit...");
        std::io::stdin().read_line(&mut String::new()).unwrap();

        Ok(())
    }

    #[test]
    fn test_ringbuf_f32() -> Result<(), anyhow::Error> {
        let (input_device, output_device, config) = setup_host()?;

        let buffer_samples = config.sample_rate.0 as usize / 100 * config.channels as usize;
        let ring_buffer = HeapRb::<f32>::new(buffer_samples);
        let (mut producer, mut consumer) = ring_buffer.split();

        // Input stream callback for `f32` samples
        let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                if producer.try_push(sample).is_err() {
                    eprintln!("Warning: output stream fell behind; sample dropped.");
                }
            }
        };

        // Output stream callback for `f32` samples
        let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in output {
                *sample = consumer.try_pop().unwrap_or(0.0);
            }
        };

        // Build and run streams
        let input_stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
        let output_stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;

        input_stream.play()?;
        output_stream.play()?;

        println!("Streaming `f32` with ring buffer. Press enter to exit...");
        std::io::stdin().read_line(&mut String::new()).unwrap();

        Ok(())
    }

    #[test]
    fn test_ringbuf_f32_slice() -> Result<(), anyhow::Error> {
        let (input_device, output_device, config) = setup_host()?;

        let buffer_samples = config.sample_rate.0 as usize / 100 * config.channels as usize;
        let ring_buffer = HeapRb::<Vec<f32>>::new(buffer_samples);
        let (mut producer, mut consumer) = ring_buffer.split();

        // Input stream callback for `[f32]` slices
        let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if producer.try_push(data.to_vec()).is_err() {
                eprintln!("Warning: output stream fell behind; slice dropped.");
            }
        };

        // Output stream callback for `[f32]` slices
        let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if let Some(data) = consumer.try_pop() {
                for (i, sample) in data.iter().enumerate().take(output.len()) {
                    output[i] = *sample;
                }
            }
        };

        // Build and run streams
        let input_stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
        let output_stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;

        input_stream.play()?;
        output_stream.play()?;

        println!("Streaming `[f32]` with ring buffer. Press enter to exit...");
        std::io::stdin().read_line(&mut String::new()).unwrap();

        Ok(())
    }

    // Error callback for handling errors in the audio streams
    fn err_fn(err: cpal::StreamError) {
        eprintln!("An error occurred on stream: {}", err);
    }
}

fn main() {
    println!("Run tests using `cargo test`");
}
