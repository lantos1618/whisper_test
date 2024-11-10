use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Style};
use tui::widgets::{Axis, Block, Borders, Chart, Dataset, Paragraph};
use tui::Terminal;
use kanal::bounded;
use std::sync::atomic::{AtomicBool, Ordering};
use ctrlc;

// Include the generated bindings
// you need to enable vscode rust-analyzer.cargo.runBuildScripts to run this
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Define a struct to encapsulate the Opus encoder
struct SafeOpusEncoder {
    encoder: *mut OpusEncoder,
}

unsafe impl Send for SafeOpusEncoder {}

impl SafeOpusEncoder {
    fn new(sample_rate: i32, channels: i32) -> Self {
        let encoder = unsafe { opus_encoder_create(sample_rate, channels, OPUS_APPLICATION_AUDIO as i32, &mut 0) };
        SafeOpusEncoder { encoder }
    }

    fn encode(&self, pcm_data: &[i16], opus_buffer: &mut [u8]) -> i32 {
        unsafe {
            opus_encode(self.encoder, pcm_data.as_ptr(), FRAME_SIZE, opus_buffer.as_mut_ptr(), opus_buffer.len() as i32)
        }
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
    fn new(sample_rate: i32, channels: i32) -> Self {
        let decoder = unsafe { opus_decoder_create(sample_rate, channels, &mut 0) };
        SafeOpusDecoder { decoder }
    }

    fn decode(&self, opus_buffer: &[u8], pcm_out: &mut [i16]) -> i32 {
        unsafe {
            opus_decode(self.decoder, opus_buffer.as_ptr(), opus_buffer.len() as i32, pcm_out.as_mut_ptr(), FRAME_SIZE, 0)
        }
    }
}

impl Drop for SafeOpusDecoder {
    fn drop(&mut self) {
        unsafe {
            opus_decoder_destroy(self.decoder);
        }
    }
}

const FRAME_SIZE: i32 = 960; // Define FRAME_SIZE at the top

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn setup_audio_stream(
    pcm_tx: kanal::Sender<Vec<i16>>,
    max_db_value: Arc<Mutex<f32>>,
    tx: kanal::Sender<(Vec<f32>, Vec<f32>)>,
    config: cpal::StreamConfig,
    device: cpal::Device,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Convert f32 samples to i16 for Opus encoding
            let pcm_data: Vec<i16> = data.iter().map(|&x| (x * 32767.0) as i16).collect();
            pcm_tx.send(pcm_data).unwrap();

            // Increase the FFT size by zero-padding the input data
            let fft_size = 8192;
            let mut buffer: Vec<Complex<f32>> = data.iter().map(|&x| Complex::new(x, 0.0)).collect();
            buffer.resize(fft_size, Complex::new(0.0, 0.0)); // Zero-padding

            // Create an FFT planner and perform the FFT
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(buffer.len());
            fft.process(&mut buffer);

            // Calculate magnitudes and frequencies
            let magnitudes: Vec<f32> = buffer.iter().map(|c| c.norm()).collect();
            let sample_rate = config.sample_rate.0 as f32;
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
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn encoding_thread(
    opus_encoder: Arc<Mutex<SafeOpusEncoder>>,
    pcm_rx: kanal::Receiver<Vec<i16>>,
    encoded_tx: kanal::Sender<Vec<u8>>,
) {
    while let Ok(pcm_data) = pcm_rx.recv() {
        let mut opus_buffer = vec![0; 4000];
        let encoded_bytes = opus_encoder.lock().unwrap().encode(&pcm_data, &mut opus_buffer);
        if encoded_bytes > 0 {
            encoded_tx.send(opus_buffer[..encoded_bytes as usize].to_vec()).unwrap();
        }
    }
}

fn decoding_thread(
    opus_decoder: Arc<Mutex<SafeOpusDecoder>>,
    encoded_rx: kanal::Receiver<Vec<u8>>,
) {
    while let Ok(encoded_data) = encoded_rx.recv() {
        let mut pcm_out = vec![0; FRAME_SIZE as usize];
        opus_decoder.lock().unwrap().decode(&encoded_data, &mut pcm_out);
        // Process or play back pcm_out
    }
}

fn plotting_thread(
    rx: kanal::Receiver<(Vec<f32>, Vec<f32>)>,
    terminal: Arc<Mutex<Terminal<CrosstermBackend<std::io::Stdout>>>>,
    max_db_value: Arc<Mutex<f32>>,
    running: Arc<AtomicBool>,
    frame_delay: Duration,
    fps: u128,
) {
    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        // Try to receive data from the audio processing thread without blocking
        if let Ok((frequencies, magnitudes)) = rx.recv() {
            let data: Vec<(f64, f64)> = frequencies.iter().zip(magnitudes.iter()).map(|(&f, &m)| (f as f64, m as f64)).collect(); // Store in a variable
            let mut terminal = terminal.lock().unwrap(); // Lock the terminal for drawing
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
                    .split(f.size());

                // Get the current max decibel value
                let max_db = *max_db_value.lock().unwrap();
                // Calculate the y-axis upper bound based on max_db
                let y_axis_upper_bound = (max_db / 20.0).exp(); // Convert dB to linear scale

                let chart = Chart::new(vec![
                    Dataset::default()
                        .name("Frequency Spectrum")
                        .marker(tui::symbols::Marker::Dot)
                        .style(Style::default().fg(Color::Cyan))
                        .data(&data), // Use the stored data
                ])
                .block(Block::default().title("Frequency Spectrum").borders(Borders::ALL))
                .x_axis(Axis::default().title("Frequency").bounds([0.0, 2400.0]))
                .y_axis(Axis::default().title("Magnitude").bounds([0.0, y_axis_upper_bound.into()]));

                f.render_widget(chart, chunks[0]);

                let text = Paragraph::new(format!("Max dB: {:.2}\nFPS: {}", max_db, fps))
                    .block(Block::default().title("Info").borders(Borders::ALL));
                f.render_widget(text, chunks[1]);
            }).unwrap();
        }

        // Sleep for the frame delay
        thread::sleep(frame_delay);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let terminal = setup_terminal()?;
    let host = cpal::default_host();
    let device = host.default_input_device().expect("Failed to get input device");
    let config = device.default_input_config().expect("Failed to get default config");

    // Shared state for the maximum decibel value
    let max_db_value = Arc::new(Mutex::new(f32::NEG_INFINITY));

    // Kanal channel for sending data to the plotting thread
    let (tx, rx) = bounded(10);

    // Create a channel for passing PCM data to the encoding thread
    let (pcm_tx, pcm_rx) = kanal::bounded::<Vec<i16>>(10);

    // Create a channel for passing encoded data to the decoding thread
    let (encoded_tx, encoded_rx) = kanal::bounded(10);

    // Initialize Opus encoder and decoder
    let sample_rate = config.sample_rate().0 as i32;
    let channels = 1; // Assuming mono audio
    let opus_encoder = Arc::new(Mutex::new(SafeOpusEncoder::new(sample_rate, channels)));
    let opus_decoder = Arc::new(Mutex::new(SafeOpusDecoder::new(sample_rate, channels)));

    // Spawn threads
    thread::spawn({
        let opus_encoder = Arc::clone(&opus_encoder);
        move || encoding_thread(opus_encoder, pcm_rx, encoded_tx)
    });
    thread::spawn({
        let opus_decoder = Arc::clone(&opus_decoder);
        move || decoding_thread(opus_decoder, encoded_rx)
    });

    // Flag to indicate when to exit
    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);

    // Setup Ctrl+C handler
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    let stream = setup_audio_stream(pcm_tx, Arc::clone(&max_db_value), tx, config.clone().into(), device)?;

    stream.play().expect("Failed to play stream");

    // 60 fps = 16.6' ms, 100 fps = 10 ms, 120 fps = 8.3 ms
    let frame_delay = Duration::from_millis(8);
    let fps = 1000 / frame_delay.as_millis(); // Calculate FPS

    // Plotting and ASCII GIF display loop
    let terminal = Arc::new(Mutex::new(terminal)); // Wrap terminal in Arc<Mutex<>>
    thread::spawn({
        let terminal = Arc::clone(&terminal);
        let max_db_value = Arc::clone(&max_db_value);
        let running = Arc::clone(&running);
        move || plotting_thread(rx, terminal, max_db_value, running, frame_delay, fps)
    });

    // Run indefinitely
    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        if event::poll(frame_delay)? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.lock().unwrap().backend_mut(), // Lock the terminal for restoration
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.lock().unwrap().show_cursor()?; // Lock the terminal for cursor display

    // Clean up Opus encoder and decoder
    unsafe {
        let encoder = opus_encoder.lock().unwrap();
        opus_encoder_destroy(encoder.encoder);

        let decoder = opus_decoder.lock().unwrap();
        opus_decoder_destroy(decoder.decoder);
    }

    Ok(())
}
