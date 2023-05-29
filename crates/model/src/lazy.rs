use crate::types::Hex;
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use std::sync::Arc;

pub static CHAIN_ID: Lazy<ArcSwap<u64>> =
    Lazy::new(|| ArcSwap::from_pointee(Default::default()));
pub static PROTOCOL_VERSION: Lazy<ArcSwap<Hex>> =
    Lazy::new(|| ArcSwap::from_pointee(Default::default()));

pub fn set_chain_id(id: u64) {
    CHAIN_ID.store(Arc::from(id));
}
