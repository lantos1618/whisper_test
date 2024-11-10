use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Failed to initialize audio device: {0}")]
    DeviceInitError(String),

    #[error("Audio stream error: {0}")]
    StreamError(String),

    #[error("Opus encoding error: {0}")]
    OpusEncodeError(i32),

    #[error("Opus decoding error: {0}")]
    OpusDecodeError(i32),

    #[error("Channel send error: {0}")]
    ChannelError(String),

    #[error("Terminal error: {0}")]
    TerminalError(String),
} 