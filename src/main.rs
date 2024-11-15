use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ctrlc;
use kanal::{bounded, Receiver, Sender};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Style};
use tui::widgets::{Axis, Block, Borders, Chart, Dataset, Paragraph};
use tui::Terminal;
mod error;
use error::AudioError;

// Include the generated bindings
// you need to enable vscode rust-analyzer.cargo.runBuildScripts to run this
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Define a struct to encapsulate the Opus encoder
const FRAME_SIZE: i32 = 960; // Define FRAME_SIZE at the top
struct SafeOpusEncoder {
    encoder: *mut OpusEncoder,
}

unsafe impl Send for SafeOpusEncoder {}

impl SafeOpusEncoder {
    fn new(sample_rate: i32, channels: i32) -> Result<Self> {
        let mut error = 0;
        let encoder = unsafe {
            opus_encoder_create(
                sample_rate,
                channels,
                OPUS_APPLICATION_AUDIO as i32,
                &mut error,
            )
        };

        if error != 0 {
            return Err(AudioError::OpusEncodeError(error))
                .context("Failed to create Opus encoder");
        }

        Ok(SafeOpusEncoder { encoder })
    }

    fn encode(&self, pcm_data: &[i16], opus_buffer: &mut [u8]) -> Result<i32> {
        let result = unsafe {
            opus_encode(
                self.encoder,
                pcm_data.as_ptr(),
                FRAME_SIZE,
                opus_buffer.as_mut_ptr(),
                opus_buffer.len() as i32,
            )
        };

        if result < 0 {
            return Err(AudioError::OpusEncodeError(result)).context("Failed to encode audio data");
        }
        Ok(result)
    }
}

impl Drop for SafeOpusEncoder {
    fn drop(&mut self) {
        unsafe {
            opus_encoder_destroy(self.encoder);
        }
    }
}

// Define a struct to encapsulate the Opus decoder
struct SafeOpusDecoder {
    decoder: *mut OpusDecoder,
}

unsafe impl Send for SafeOpusDecoder {}

impl SafeOpusDecoder {
    fn new(sample_rate: i32, channels: i32) -> Result<Self> {
        let mut error = 0;
        let decoder = unsafe { opus_decoder_create(sample_rate, channels, &mut error) };

        if error != 0 {
            return Err(AudioError::OpusDecodeError(error))
                .context("Failed to create Opus decoder");
        }

        Ok(SafeOpusDecoder { decoder })
    }

    fn decode(&self, opus_buffer: &[u8], pcm_out: &mut [i16]) -> Result<i32> {
        let result = unsafe {
            opus_decode(
                self.decoder,
                opus_buffer.as_ptr(),
                opus_buffer.len() as i32,
                pcm_out.as_mut_ptr(),
                FRAME_SIZE,
                0,
            )
        };

        if result < 0 {
            return Err(AudioError::OpusDecodeError(result)).context("Failed to decode audio data");
        }
        Ok(result)
    }
}

impl Drop for SafeOpusDecoder {
    fn drop(&mut self) {
        unsafe {
            opus_decoder_destroy(self.decoder);
        }
    }
}

fn audio_input(tx: Sender<f32>) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_input_device().context("No input device available")?;
    let config = device.default_input_config().context("No input config available")?;

    let channels = config.channels() as usize;

    let input_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        for &sample in data.iter() {
            if tx.try_send(sample).is_err() {
                eprintln!("Warning: output stream fell behind; sample dropped.");
            }
        }
    };

    let stream = device.build_input_stream(
        &config.into(),
        input_callback,
        |err| {
            eprintln!("Audio stream error: {}", err);
        },
        None,
    )?;

    stream.play()?;

    // Keep the stream alive
    loop {
        thread::sleep(Duration::from_millis(10));
    }
}

fn audio_output(rx: Receiver<f32>) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_output_device().context("No output device available")?;
    let config = device.default_output_config().context("No output config available")?;

    let channels = config.channels() as usize;

    let output_callback = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
        for frame in output.chunks_mut(channels) {
            let sample = rx.try_recv().unwrap();
            for sample_channel in frame.iter_mut() {
                match sample {
                    Some(sample) => *sample_channel = sample,
                    None => *sample_channel = 0.0,
                }
            }
        }
    };

    let stream = device.build_output_stream(
        &config.into(),
        output_callback,
        |err| {
            eprintln!("Audio stream error: {}", err);
        },
        None,
    )?;

    stream.play()?;

    // Keep the stream alive
    loop {
        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}

fn main() -> Result<()> {
    let (input_tx, input_rx) = bounded(1024); // Adjust buffer size as necessary

    let handles = vec![
        thread::spawn(move || audio_input(input_tx)),
        thread::spawn(move || audio_output(input_rx)),
    ];

    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}
