use rlp_derive::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

use crate::codec::ProtocolCodec;
use crate::types::{
    Bloom, BloomInput, Bytes, ExecResp, Hash, Hasher, MerkleRoot, SignedTransaction,
    H160, H64, RLP_NULL, U256,
};

pub type BlockNumber = u64;

pub const MAX_BLOCK_GAS_LIMIT: u64 = 50_000_000;
pub const BASE_FEE_PER_GAS: u64 = 0x539;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub prev_hash: Hash,
    pub proposer: H160,
    pub transactions_root: MerkleRoot,
    pub timestamp: u64,
    pub number: BlockNumber,
    pub gas_limit: U256,
    pub extra_data: Bytes,
    pub mixed_hash: Option<Hash>,
    pub base_fee_per_gas: U256,
    pub chain_id: u64,
    pub tx_hashes: Vec<Hash>,
}

impl From<&Block> for Proposal {
    fn from(b: &Block) -> Self {
        Self::from(&b.header)
    }
}

impl From<&Header> for Proposal {
    fn from(h: &Header) -> Self {
        Proposal {
            prev_hash: h.prev_hash,
            proposer: h.proposer,
            transactions_root: h.transactions_root,
            timestamp: h.timestamp,
            number: h.number,
            gas_limit: h.gas_limit,
            extra_data: h.extra_data.clone(),
            mixed_hash: h.mixed_hash,
            base_fee_per_gas: h.base_fee_per_gas,
            chain_id: h.chain_id,
            tx_hashes: vec![],
        }
    }
}

impl From<Header> for Proposal {
    fn from(h: Header) -> Self {
        Proposal {
            prev_hash: h.prev_hash,
            proposer: h.proposer,
            transactions_root: h.transactions_root,
            timestamp: h.timestamp,
            number: h.number,
            gas_limit: h.gas_limit,
            extra_data: h.extra_data,
            mixed_hash: h.mixed_hash,
            base_fee_per_gas: h.base_fee_per_gas,
            chain_id: h.chain_id,
            tx_hashes: vec![],
        }
    }
}

impl Proposal {
    pub fn hash(&self) -> Hash {
        Hasher::digest(self.encode().unwrap())
    }
}

pub struct PackedTxHashes {
    pub hashes: Vec<Hash>,
    pub call_system_script_count: u32,
}

#[derive(
    RlpEncodable,
    RlpDecodable,
    Serialize,
    Deserialize,
    Default,
    Clone,
    Debug,
    PartialEq,
    Eq,
)]
pub struct Block {
    pub header: Header,
    pub tx_hashes: Vec<Hash>,
}

impl Block {
    pub fn new(proposal: Proposal, exec_resp: &ExecResp) -> Self {
        let logs = exec_resp
            .txs_resp
            .iter()
            .map(|r| Bloom::from(BloomInput::Raw(rlp::encode_list(&r.logs).as_ref())))
            .collect::<Vec<_>>();
        let header = Header {
            prev_hash: proposal.prev_hash,
            proposer: proposal.proposer,
            state_root: exec_resp.state_root,
            transactions_root: proposal.transactions_root,
            receipts_root: exec_resp.receipt_root,
            log_bloom: Bloom::from(BloomInput::Raw(rlp::encode_list(&logs).as_ref())),
            difficulty: U256::one(),
            timestamp: proposal.timestamp,
            number: proposal.number,
            gas_used: exec_resp.gas_used.into(),
            gas_limit: proposal.gas_limit,
            extra_data: proposal.extra_data,
            mixed_hash: proposal.mixed_hash,
            nonce: Default::default(),
            base_fee_per_gas: proposal.base_fee_per_gas,
            chain_id: proposal.chain_id,
        };

        Block {
            header,
            tx_hashes: proposal.tx_hashes,
        }
    }

    pub fn hash(&self) -> Hash {
        Proposal::from(self).hash()
    }

    pub fn genesis(chain_id: u64) -> Self {
        let header = Header {
            prev_hash: Default::default(),
            proposer: Default::default(),
            state_root: Default::default(),
            transactions_root: RLP_NULL,
            receipts_root: RLP_NULL,
            log_bloom: Bloom::default(),
            difficulty: U256::one(),
            timestamp: ruc::ts!(),
            number: 0,
            gas_used: 0.into(),
            gas_limit: MAX_BLOCK_GAS_LIMIT.into(),
            extra_data: Default::default(),
            mixed_hash: None,
            nonce: Default::default(),
            base_fee_per_gas: BASE_FEE_PER_GAS.into(),
            chain_id,
        };

        Block {
            header,
            tx_hashes: vec![],
        }
    }
}

#[derive(
    RlpEncodable,
    RlpDecodable,
    Serialize,
    Deserialize,
    Default,
    Clone,
    Debug,
    PartialEq,
    Eq,
)]
pub struct Header {
    pub prev_hash: Hash,
    pub proposer: H160,
    pub state_root: MerkleRoot,
    pub transactions_root: MerkleRoot,
    pub receipts_root: MerkleRoot,
    pub log_bloom: Bloom,
    pub difficulty: U256,
    pub timestamp: u64,
    pub number: BlockNumber,
    pub gas_used: U256,
    pub gas_limit: U256,
    pub extra_data: Bytes,
    pub mixed_hash: Option<Hash>,
    pub nonce: H64,
    pub base_fee_per_gas: U256,
    pub chain_id: u64,
}

impl Header {
    pub fn size(&self) -> usize {
        self.encode().unwrap().len()
    }

    pub fn hash(&self) -> Hash {
        Proposal::from(self).hash()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RichBlock {
    pub block: Block,
    pub txs: Vec<SignedTransaction>,
}
