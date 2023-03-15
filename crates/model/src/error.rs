use derive_more::{Constructor, Display};
use std::error::Error;

#[derive(Debug, Clone)]
pub enum ProtocolErrorKind {
    API,
    Executor,
    Storage,

    Types,
    Codec,

    System,
    Unknown,
}

#[derive(Debug, Constructor, Display)]
#[display(fmt = "ProtocolError, Kind: {:?}", kind)]
pub struct ProtocolError {
    kind: ProtocolErrorKind,
}

impl ProtocolError {
    pub fn api() -> Self {
        Self {
            kind: ProtocolErrorKind::API,
        }
    }

    pub fn executor() -> Self {
        Self {
            kind: ProtocolErrorKind::Executor,
        }
    }

    pub fn storage() -> Self {
        Self {
            kind: ProtocolErrorKind::Storage,
        }
    }

    pub fn types() -> Self {
        Self {
            kind: ProtocolErrorKind::Types,
        }
    }

    pub fn codec() -> Self {
        Self {
            kind: ProtocolErrorKind::Codec,
        }
    }

    pub fn system() -> Self {
        Self {
            kind: ProtocolErrorKind::System,
        }
    }

    pub fn unknown() -> Self {
        Self {
            kind: ProtocolErrorKind::Unknown,
        }
    }
}

impl From<ProtocolError> for Box<dyn Error + Send> {
    fn from(error: ProtocolError) -> Self {
        Box::new(error) as Box<dyn Error + Send>
    }
}

impl From<ProtocolError> for String {
    fn from(error: ProtocolError) -> String {
        error.to_string()
    }
}

impl Error for ProtocolError {}
