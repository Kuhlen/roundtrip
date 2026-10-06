#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("DNS resolution failed: {0}")]
    DnsFailure(String),
    #[error("connection refused: {0}")]
    ConnectionRefused(String),
    #[error("timeout after {0} ms")]
    Timeout(u64),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("response too large (> {0} bytes)")]
    ResponseTooLarge(u64),
    #[error("not a collection: {0}")]
    NotACollection(String),
    #[error("merge conflict in {0}")]
    MergeConflict(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("storage error: {0}")]
    Storage(String),
}
