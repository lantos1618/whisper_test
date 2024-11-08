use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("Failed to get input device");
    let config = device.default_input_config().expect("Failed to get default config");

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Process audio data in `data`, which contains float samples
            // Feed this data to your diarization/ASR system
        },
        err_fn,
        None,
    ).expect("Failed to build input stream");

    stream.play().expect("Failed to play stream");

    std::thread::sleep(std::time::Duration::from_secs(10));
}