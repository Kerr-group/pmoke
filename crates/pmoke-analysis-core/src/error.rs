use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisError {
    code: &'static str,
    message: String,
}

impl AnalysisError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for AnalysisError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AnalysisError {}

pub type Result<T> = std::result::Result<T, AnalysisError>;
