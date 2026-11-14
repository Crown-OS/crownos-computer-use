#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("malformed data from the source: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the app's MCP server said: {0}")]
    Mcp(String),
    #[error("AT-SPI: {0}")]
    Atspi(String),
    #[error("the browser said: {0}")]
    Browser(String),
    #[error("the source cannot {0} this element")]
    Unsupported(&'static str),
    #[error("the source went away")]
    Gone,
    #[error("the browser did not answer in time")]
    Timeout,
}

impl From<zbus::Error> for SourceError {
    fn from(error: zbus::Error) -> Self {
        Self::Atspi(error.to_string())
    }
}

impl From<atspi::AtspiError> for SourceError {
    fn from(error: atspi::AtspiError) -> Self {
        Self::Atspi(error.to_string())
    }
}
