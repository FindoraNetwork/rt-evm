pub mod block;
pub mod error;
pub mod executor;
pub mod receipt;
pub mod transaction;

use crate::types::{Address, Bytes, DBBytes, Hex, TypesError};
use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};
use ruc::*;
use std::result::Result as StdResult;

pub trait ProtocolCodec: Sized + Send {
    fn encode(&self) -> Result<Bytes>;
    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self>;
}

impl<T: Encodable + Decodable + Send> ProtocolCodec for T {
    fn encode(&self) -> Result<Bytes> {
        Ok(self.rlp_bytes().to_vec())
    }

    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self> {
        Self::decode(&Rlp::new(bytes.as_ref()))
            .map_err(|e| error::CodecError::Rlp(e.to_string()))
            .c(d!())
    }
}

impl ProtocolCodec for DBBytes {
    fn encode(&self) -> Result<Bytes> {
        Ok(self.0.clone())
    }

    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self> {
        let inner = bytes.as_ref().to_vec();
        Ok(Self(inner))
    }
}

impl Encodable for Address {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(1).append(&self.0);
    }
}

impl Decodable for Address {
    fn decode(r: &Rlp) -> StdResult<Self, DecoderError> {
        Ok(Address(r.val_at(0)?))
    }
}

impl Encodable for Hex {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(1).append(&self.as_string_trim0x());
    }
}

impl Decodable for Hex {
    fn decode(r: &Rlp) -> StdResult<Self, DecoderError> {
        Hex::from_string(r.val_at(0)?).map_err(|_| DecoderError::Custom("hex check"))
    }
}

pub fn hex_encode<T: AsRef<[u8]>>(src: T) -> String {
    faster_hex::hex_string(src.as_ref())
}

pub fn hex_decode(src: &str) -> Result<Vec<u8>> {
    if src.is_empty() {
        return Ok(Vec::new());
    }

    let src = if src.starts_with("0x") {
        src.split_at(2).1
    } else {
        src
    };

    let src = src.as_bytes();
    let mut ret = vec![0u8; src.len() / 2];
    faster_hex::hex_decode(src, &mut ret)
        .map_err(TypesError::FromHex)
        .c(d!())?;

    Ok(ret)
}
