//! Crate-wide error type.

use std::fmt;

/// Error type used across the workspace crates.
#[derive(Debug)]
pub enum MapServerError {
    /// I/O (file, range read, network).
    Io(std::io::Error),
    /// A malformed container / archive / payload (COG, PMTiles, ...).
    Format(String),
    /// A bad client request (WMS/WMTS parameter errors).
    BadRequest(String),
    /// Any other internal error.
    Other(String),
}

impl fmt::Display for MapServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapServerError::Io(e) => write!(f, "io error: {e}"),
            MapServerError::Format(m) => write!(f, "format error: {m}"),
            MapServerError::BadRequest(m) => write!(f, "bad request: {m}"),
            MapServerError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for MapServerError {}

impl From<std::io::Error> for MapServerError {
    fn from(e: std::io::Error) -> Self {
        MapServerError::Io(e)
    }
}

impl From<String> for MapServerError {
    fn from(m: String) -> Self {
        MapServerError::Other(m)
    }
}

impl From<&str> for MapServerError {
    fn from(m: &str) -> Self {
        MapServerError::Other(m.to_string())
    }
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, MapServerError>;
