use crate::{
    lazy::CHAIN_ID,
    types::{Proposal, BASE_FEE_PER_GAS, MAX_BLOCK_GAS_LIMIT},
};
use rlp::{Decodable, DecoderError, Encodable, Prototype, Rlp, RlpStream};

impl Encodable for Proposal {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(6)
            .append(&self.prev_hash)
            .append(&self.proposer)
            .append(&self.transactions_root)
            .append(&self.timestamp)
            .append(&self.number)
            .append_list(&self.tx_hashes);
    }
}

impl Decodable for Proposal {
    fn decode(r: &Rlp) -> Result<Self, DecoderError> {
        match r.prototype()? {
            Prototype::List(10) => Ok(Proposal {
                prev_hash: r.val_at(0)?,
                proposer: r.val_at(1)?,
                transactions_root: r.val_at(2)?,
                timestamp: r.val_at(3)?,
                number: r.val_at(4)?,
                gas_limit: MAX_BLOCK_GAS_LIMIT.into(),
                extra_data: Default::default(),
                mixed_hash: None,
                base_fee_per_gas: BASE_FEE_PER_GAS.into(),
                chain_id: **CHAIN_ID.load(),
                tx_hashes: r.list_at(5)?,
            }),
            _ => Err(DecoderError::RlpInconsistentLengthAndData),
        }
    }
}
