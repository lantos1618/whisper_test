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
use cpal::Stream;
use error::AudioError;

// Include the generated bindings
// you need to enable vscode rust-analyzer.cargo.runBuildScripts to run this
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Define a struct to encapsulate the Opus encoder

const MAX_PACKET_SIZE: usize = 1275; // Maximum size of an Opus packet for 48kHz stereo
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
                OPUS_APPLICATION_VOIP as i32,
                &mut error,
            )
        };

        if error != 0 {
            return Err(AudioError::OpusEncodeError(error))
                .context("Failed to create Opus encoder");
        }

        Ok(SafeOpusEncoder { encoder })
    }

    fn encode(&self, pcm_data: &[i16], opus_buffer: &mut [u8], frame_size: i32) -> Result<i32> {
        let result = unsafe {
            opus_encode(
                self.encoder,
                pcm_data.as_ptr(),
                frame_size,
                opus_buffer.as_mut_ptr(),
                opus_buffer.len() as i32,
            )
        };

        if result < 0 {
            return Err(AudioError::OpusEncodeError(result)).context("Failed to encode audio data");
        }
        Ok(result)
    }
    fn encode_float(&self, pcm_data: &[f32], opus_buffer: &mut [u8], frame_size: i32) -> Result<i32> {
        let result = unsafe {
            opus_encode_float(
                self.encoder,
                pcm_data.as_ptr(),
                frame_size,
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

    fn decode(&self, opus_buffer: &[u8], pcm_out: &mut [i16], frame_size: i32) -> Result<i32> {
        let result = unsafe {
            opus_decode(
                self.decoder,
                opus_buffer.as_ptr(),
                opus_buffer.len() as i32,
                pcm_out.as_mut_ptr(),
                frame_size,
                0,
            )
        };

        if result < 0 {
            return Err(AudioError::OpusDecodeError(result)).context("Failed to decode audio data");
        }
        Ok(result)
    }

    fn decode_float(&self, opus_buffer: &[u8], pcm_out: &mut [f32], frame_size: i32) -> Result<i32> {
        let result = unsafe {
            opus_decode_float(
                self.decoder,
                opus_buffer.as_ptr(),
                opus_buffer.len() as i32,
                pcm_out.as_mut_ptr(),
                frame_size,
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

fn setup_host() -> Result<(cpal::Device, cpal::Device, cpal::StreamConfig, i32)> {
    let host = cpal::default_host();

    let input_device = host
        .default_input_device()
        .ok_or_else(|| AudioError::NoDevice("No input device found".into()))?;

    let output_device = host
        .default_output_device()
        .ok_or_else(|| AudioError::NoDevice("No output device found".into()))?;

    let config = input_device
        .default_input_config()
        .map_err(|e| AudioError::StreamConfigError(e.to_string()))?;
    
    // Calculate frame size based on sample rate (20ms frame size)
    let frame_size = (config.sample_rate().0 as f32 * 0.02) as i32;

    Ok((input_device, output_device, config.into(), frame_size))
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {}", err);
}

fn audio_input(running: Arc<AtomicBool>, tx: Sender<Vec<f32>>) -> Result<()> {
    let (input_device, _output_device, config, _frame_size) = setup_host()?;

    let input_data_fn =
        move |data: &[f32], _: &cpal::InputCallbackInfo| match tx.try_send(data.to_vec()) {
            Ok(_) => (),
            Err(e) => eprintln!("Error audio_input: {:?}", e),
        };

    let stream = input_device.build_input_stream(&config, input_data_fn, err_fn, None)?;
    stream.play()?;

    while running.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}

fn audio_output(running: Arc<AtomicBool>, rx: Receiver<Vec<f32>>) -> Result<()> {
    let (_input_device, output_device, config, _frame_size) = setup_host()?;

    let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
        match rx.try_recv() {
            Ok(val) => match val {
                Some(data) => {
                    for (i, sample) in output.iter_mut().enumerate().take(data.len()) {
                        *sample = data[i];
                    }
                }
                None => (),
            },
            Err(e) => eprintln!("Error audio_output: {:?}", e),
        };
    };
    let stream = output_device.build_output_stream(&config, output_data_fn, err_fn, None)?;
    stream.play()?;

    while running.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}

fn encode_audio(
    running: Arc<AtomicBool>,
    rx: Receiver<Vec<f32>>,
    tx: Sender<Vec<u8>>,
    frame_size: i32,
) -> Result<()> {
    let encoder = SafeOpusEncoder::new(48000, 1)?;
    let mut opus_buffer = vec![0u8; MAX_PACKET_SIZE];

    while running.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(val) => {
                match val {
                    Some(data) => {
                        let encoded_len = encoder.encode_float(&data, &mut opus_buffer, frame_size)?;
                        tx.send(opus_buffer[..encoded_len as usize].to_vec())?;
                    }
                    None => (),
                }
            }
            Err(e) => eprintln!("Error encode_audio: {:?}", e),
        }
    }
    Ok(())
}

fn decode_audio(
    running: Arc<AtomicBool>,
    rx: Receiver<Vec<u8>>,
    tx: Sender<Vec<f32>>,
    frame_size: i32,
) -> Result<()> {
    let decoder = SafeOpusDecoder::new(48000, 1)?;

    while running.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(val) => match val {
                Some(data) => {
                    let mut pcm_out = vec![0.0; frame_size as usize];
                    let decoded_len = decoder.decode_float(&data, &mut pcm_out, frame_size)?;
                    tx.send(pcm_out[..decoded_len as usize].to_vec())?;
                }
                None => (),
            },
            Err(e) => eprintln!("Error decode_audio: {:?}", e),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let (input_tx, input_rx) = bounded(16);
    let (encoder_tx, encoder_rx) = bounded(16);
    let (decoder_tx, decoder_rx) = bounded(16);

    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = running.clone();

    // Get frame_size from host setup
    let (_input_device, _output_device, _config, frame_size) = setup_host()?;

    // Set up Ctrl+C handler
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C! Shutting down...");
        running_ctrlc.store(false, Ordering::Relaxed);
    })?;

    let handles = vec![
        {
            let running = running.clone();
            thread::spawn(move || audio_input(running, input_tx))
        },
        {
            let running = running.clone();
            thread::spawn(move || encode_audio(running, input_rx, encoder_tx, frame_size))
        },
        {
            let running = running.clone();
            thread::spawn(move || decode_audio(running, encoder_rx, decoder_tx, frame_size))
        },
        {
            let running = running.clone();
            thread::spawn(move || audio_output(running, decoder_rx))
        },
    ];

    for handle in handles {
        handle.join().unwrap()?;
    }

    Ok(())
}
