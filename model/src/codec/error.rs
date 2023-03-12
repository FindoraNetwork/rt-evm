use derive_more::Display;
use std::error::Error;

#[derive(Debug, Display)]
pub enum CodecError {
    #[display(fmt = "rlp: from string {}", _0)]
    Rlp(String),
}

impl Error for CodecError {}
