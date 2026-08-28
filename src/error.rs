use std::io;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CacheError {
    pub code: &'static str,
    message: String,
}

impl CacheError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn io(code: &'static str, error: io::Error) -> Self {
        Self::new(code, error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CacheError>;

pub trait IoResultExt<T> {
    fn cache_err(self, code: &'static str) -> Result<T>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn cache_err(self, code: &'static str) -> Result<T> {
        self.map_err(|error| CacheError::io(code, error))
    }
}
