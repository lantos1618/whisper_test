use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("No device available: {0}")]
    NoDevice(String),
    
    #[error("Stream configuration error: {0}")]
    StreamConfigError(String),
    
    #[error("Opus encode error: {0}")]
    OpusEncodeError(i32),
    
    #[error("Opus decode error: {0}")]
    OpusDecodeError(i32),
} 