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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .expect("Failed to get input device");
    let config = device
        .default_input_config()
        .expect("Failed to get default config");

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    // Shared state for the maximum decibel value
    let max_db_value = Arc::new(Mutex::new(f32::NEG_INFINITY));

    // Kanal channel for sending data to the plotting thread
    let (tx, rx) = bounded(10);

    let stream = device
        .build_input_stream(
            &config.clone().into(),
            {
                let max_db_value = Arc::clone(&max_db_value);
                let tx = tx.clone();
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Increase the FFT size by zero-padding the input data
                    let fft_size = 8192;
                    // let fft_size = 4096;
                    let mut buffer: Vec<Complex<f32>> =
                        data.iter().map(|&x| Complex::new(x, 0.0)).collect();
                    buffer.resize(fft_size, Complex::new(0.0, 0.0)); // Zero-padding

                    // Create an FFT planner and perform the FFT
                    let mut planner = FftPlanner::new();
                    let fft = planner.plan_fft_forward(buffer.len());
                    fft.process(&mut buffer);

                    // Calculate magnitudes and frequencies
                    let magnitudes: Vec<f32> = buffer.iter().map(|c| c.norm()).collect();
                    let sample_rate = config.sample_rate().0 as f32;
                    let frequencies: Vec<f32> = (0..buffer.len())
                        .map(|i| i as f32 * sample_rate / buffer.len() as f32)
                        .collect();

                    // Update maximum decibel value to track highest signal
                    {
                        let mut max_db_lock = max_db_value.lock().unwrap();
                        let max_magnitude =
                            magnitudes.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                        let max_db = 20.0 * max_magnitude.log10();
                        if max_db > *max_db_lock {
                            *max_db_lock = max_db;
                        }
                    }

                    // Send data to the plotting thread
                    tx.send((frequencies, magnitudes))
                        .expect("Failed to send data to plotting thread");
                }
            },
            err_fn,
            None,
        )
        .expect("Failed to build input stream");

    stream.play().expect("Failed to play stream");

    // 60 fps = 16.6' ms, 100 fps = 10 ms, 120 fps = 8.3 ms
    let frame_delay = Duration::from_millis(8);
    let fps = 1000 / frame_delay.as_millis(); // Calculate FPS

    // Plotting and ASCII GIF display loop
    let terminal = Arc::new(Mutex::new(terminal)); // Wrap terminal in Arc<Mutex<>>
    let terminal_clone = Arc::clone(&terminal);

    thread::spawn(move || {
        loop {
            // Try to receive data from the audio processing thread without blocking
            if let Ok((frequencies, magnitudes)) = rx.recv() {
                let data: Vec<(f64, f64)> = frequencies
                    .iter()
                    .zip(magnitudes.iter())
                    .map(|(&f, &m)| (f as f64, m as f64))
                    .collect(); // Store in a variable
                let mut terminal = terminal_clone.lock().unwrap(); // Lock the terminal for drawing
                terminal
                    .draw(|f| {
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .margin(1)
                            .constraints(
                                [Constraint::Percentage(70), Constraint::Percentage(30)].as_ref(),
                            )
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
                        .block(
                            Block::default()
                                .title("Frequency Spectrum")
                                .borders(Borders::ALL),
                        )
                        .x_axis(Axis::default().title("Frequency").bounds([0.0, 2400.0]))
                        .y_axis(
                            Axis::default()
                                .title("Magnitude")
                                .bounds([0.0, y_axis_upper_bound.into()]),
                        );

                        f.render_widget(chart, chunks[0]);

                        let text = Paragraph::new(format!("Max dB: {:.2}\nFPS: {}", max_db, fps))
                            .block(Block::default().title("Info").borders(Borders::ALL));
                        f.render_widget(text, chunks[1]);
                    })
                    .unwrap();
            }

            // Sleep for the frame delay
            thread::sleep(frame_delay);
        }
    });

    // Run indefinitely
    loop {
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

    Ok(())
}
