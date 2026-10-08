use std::fmt;

/// Service failures remain independent of HTTP; the API chooses status codes.
#[derive(Debug)]
pub enum Error {
    BadRequest(String),
    UnsupportedMediaType,
    UploadTooLarge,
    UploadCapacity,
    UploadTimeout,
    InvalidConstruct(String),
    NotFound,
    Internal(String),
}

impl Error {
    pub(crate) fn internal(error: impl fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(message)
            | Self::InvalidConstruct(message)
            | Self::Internal(message) => formatter.write_str(message),
            Self::UnsupportedMediaType => formatter.write_str("use application/octet-stream"),
            Self::UploadTooLarge => formatter.write_str("upload exceeds 128 MiB"),
            Self::UploadCapacity => formatter.write_str("upload capacity reached; retry later"),
            Self::UploadTimeout => formatter.write_str("upload timed out"),
            Self::NotFound => formatter.write_str("construct not found"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
