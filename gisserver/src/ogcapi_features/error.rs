//! OGC API error model: JSON `{"code", "description"}` with
//! the matching HTTP status.

use axum::http::StatusCode;

/// An OGC API problem (code + human description + HTTP status).
#[derive(Clone, Debug)]
pub struct OgcError {
    pub code: &'static str,
    pub message: String,
    pub status: StatusCode,
}

impl OgcError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "NotFound",
            message: message.into(),
            status: StatusCode::NOT_FOUND,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            code: "InvalidParameterValue",
            message: message.into(),
            status: StatusCode::BAD_REQUEST,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "InternalError",
            message: message.into(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_oaf_conventions() {
        assert_eq!(OgcError::not_found("x").status, StatusCode::NOT_FOUND);
        assert_eq!(OgcError::bad_request("x").status, StatusCode::BAD_REQUEST);
        assert_eq!(
            OgcError::internal("x").status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
