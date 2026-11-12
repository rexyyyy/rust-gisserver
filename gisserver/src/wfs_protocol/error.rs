//! WFS protocol error type + OGC ServiceException / ExceptionReport XML.

use axum::http::StatusCode;

/// A WFS exception: the OGC `code` plus a human message.
#[derive(Clone, Debug)]
pub struct WfsError {
    pub code: &'static str,
    pub message: String,
}

impl WfsError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Missing required parameter (`MissingParameterValue`).
    pub fn missing(param: &str) -> Self {
        Self::new("MissingParameterValue", format!("missing required parameter '{param}'"))
    }

    /// Bad parameter value (`InvalidParameterValue`).
    pub fn invalid(param: &str, message: impl Into<String>) -> Self {
        Self::new(
            "InvalidParameterValue",
            format!("{} ('{param}')", message.into()),
        )
    }

    /// Unsupported operation (`OperationNotSupported`).
    pub fn unsupported(op: &str) -> Self {
        Self::new("OperationNotSupported", format!("operation '{op}' is not supported"))
    }

    /// Unsupported output format (`InvalidParameterValue`).
    pub fn bad_format(fmt: &str) -> Self {
        Self::invalid("outputFormat", format!("unsupported output format '{fmt}'"))
    }

    /// Unknown feature type (`InvalidParameterValue` per WFS 2.0).
    pub fn unknown_type(name: &str) -> Self {
        Self::invalid("typeName", format!("feature type '{name}' is not published"))
    }

    /// HTTP status: protocol errors are 400, everything else 500.
    pub fn status(&self) -> StatusCode {
        match self.code {
            "MissingParameterValue" | "InvalidParameterValue" | "OperationParsingFailed"
            | "VersionNegotiationFailed" | "OperationNotSupported" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// WFS 1.x style `ServiceExceptionReport`.
pub fn service_exception_xml(err: &WfsError) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ServiceExceptionReport xmlns=\"http://www.opengis.net/ogc\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         version=\"1.2.0\">\n\
         <ServiceException code=\"{code}\">{msg}</ServiceException>\n\
         </ServiceExceptionReport>",
        code = err.code,
        msg = super::caps::xml_escape(&err.message),
    )
}

/// WFS 2.0 style `ows:ExceptionReport` (subtype-agnostic).
pub fn exception_report_xml(err: &WfsError) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ows:ExceptionReport xmlns:ows=\"http://www.opengis.net/ows/1.1\" \
         version=\"2.0.0\">\n\
         <ows:Exception exceptionCode=\"{code}\">\n\
         <ows:ExceptionText>{msg}</ows:ExceptionText>\n\
         </ows:Exception>\n\
         </ows:ExceptionReport>",
        code = err.code,
        msg = super::caps::xml_escape(&err.message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_xml_escapes_message() {
        let e = WfsError::invalid("bbox", "bad <bbox> & stuff");
        let xml = exception_report_xml(&e);
        assert!(xml.contains("exceptionCode=\"InvalidParameterValue\""), "{xml}");
        assert!(xml.contains("bad &lt;bbox&gt; &amp; stuff"), "{xml}");
        let old = service_exception_xml(&e);
        assert!(old.contains("<ServiceException code=\"InvalidParameterValue\">"), "{old}");
    }

    #[test]
    fn status_codes() {
        assert_eq!(WfsError::missing("bbox").status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            WfsError::new("NoApplicableCode", "boom").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
