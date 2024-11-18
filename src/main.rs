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

fn setup_host() -> Result<(cpal::Device, cpal::Device, cpal::StreamConfig)> {
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

    Ok((input_device, output_device, config.into()))
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {}", err);
}

fn audio_input(running: Arc<AtomicBool>, tx: Sender<Vec<f32>>) -> Result<()> {
    let (input_device, _output_device, config) = setup_host()?;

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
    let (_input_device, output_device, config) = setup_host()?;

    let output_data_fn = move |output: &mut [f32], _: &cpal::OutputCallbackInfo| match rx.try_recv()
    {
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
) -> Result<()> {
    let encoder = SafeOpusEncoder::new(48000, 1)?;
    let mut opus_buffer = vec![0; 4096];

    while running.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(val) => {
                match val {
                    Some(data) => {
                        let pcm_data: Vec<i16> = data
                            .iter()
                            .map(|&sample| (sample * 32767.0) as i16)
                            .collect();
                        let result = encoder.encode(&pcm_data, &mut opus_buffer)?;
                        println!("encode_audio: {:?}", result);
                        tx.send(opus_buffer.clone())?;

                        // // dont encode, just pass it on to test. we need to F32 to u8
                        // let u8_data: Vec<u8> = data.iter().map(|&sample| (sample * 127.0 + 127.0) as u8).collect();
                        // tx.send(u8_data.clone())?;
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
) -> Result<()> {
    let decoder = SafeOpusDecoder::new(48000, 1)?;

    while running.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(val) => match val {
                Some(data) => {
                    let mut pcm_data = vec![0; FRAME_SIZE as usize];
                    let result = decoder.decode(&data, &mut pcm_data)?;
                    println!("decode_audio: {:?}", result);
                }
                None => (),
            },
            Err(e) => eprintln!("Error decode_audio: {:?}", e),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let (input_tx, input_rx) = bounded(8192);
    let (encoder_tx, decoder_rx) = bounded(8192);
    let (decoder_tx, output_rx) = bounded(8192);

    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = running.clone();

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
            thread::spawn(move || encode_audio(running, input_rx, encoder_tx))
        },
        {
            let running = running.clone();
            thread::spawn(move || decode_audio(running, decoder_rx, decoder_tx))
        },
        {
            let running = running.clone();
            thread::spawn(move || audio_output(running, output_rx))
        },
    ];

    for handle in handles {
        handle.join().unwrap()?;
    }

    Ok(())
}
