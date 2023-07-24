#![deny(warnings)]
#![cfg_attr(feature = "benchmark", allow(warnings))]

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rayon::prelude::*;
use rt_evm_executor::{
    logs_bloom, trie_root_indexed, trie_root_txs, RTEvmExecutor as Executor,
    RTEvmExecutorAdapter as EvmExecBackend,
};
use rt_evm_mempool::Mempool;
use rt_evm_model::{
    traits::{BlockStorage as _, Executor as _, SystemContract, TxStorage as _},
    types::{
        Block, ExecResp, ExecutorContext, FatBlock, FatBlockRef, Hash, Header,
        MerkleRoot, Proposal, Receipt, SignedTransaction, BASE_FEE_PER_GAS, H160,
        MAX_BLOCK_GAS_LIMIT, U256,
    },
};
use rt_evm_storage::{MptStore, Storage};
use ruc::*;
use std::sync::Arc;

/// Seperate `evm_call` and `evm_exec`
pub static EXEC_LK: Lazy<RwLock<bool>> = Lazy::new(|| RwLock::new(true));

pub struct BlockMgmt {
    pub proposer: H160,

    // the block hash of the previous block
    pub prev_block_hash: MerkleRoot,

    // the state hash of the previous block
    pub prev_state_root: MerkleRoot,

    // the height of the proposing block
    pub block_number: u64,

    // the timestamp of the proposing block
    pub block_timestamp: u64,

    pub chain_id: u64,

    pub mempool: Arc<Mempool>,
    pub trie: Arc<MptStore>,
    pub storage: Arc<Storage>,
}

impl BlockMgmt {
    pub fn new(
        proposer: H160,
        timestamp: u64,
        mempool: Arc<Mempool>,
        trie: Arc<MptStore>,
        storage: Arc<Storage>,
    ) -> Result<Self> {
        let latest_block_header = storage.get_latest_block_header().c(d!())?;
        Ok(Self {
            proposer,
            prev_block_hash: latest_block_header.hash(),
            prev_state_root: latest_block_header.state_root,
            block_number: 1 + latest_block_header.number,
            block_timestamp: timestamp,
            chain_id: latest_block_header.chain_id,
            mempool,
            trie,
            storage,
        })
    }

    /// generate a new block and persist it
    pub fn produce_block(
        &self,
        txs: Vec<SignedTransaction>,
        system_contracts: Option<Vec<SystemContract>>,
    ) -> Result<Header> {
        let lk = EXEC_LK.write();

        let (block, receipts) = self.generate_block(&txs, system_contracts).c(d!())?;
        let header = block.header.clone();

        self.storage.insert_txs(header.number, txs).c(d!())?;
        self.storage
            .insert_receipts(header.number, receipts)
            .c(d!())?;
        self.storage.set_block(block).c(d!())?;

        assert!(*lk);

        Ok(header)
    }

    fn generate_block(
        &self,
        txs: &[SignedTransaction],
        system_contracts: Option<Vec<SystemContract>>,
    ) -> Result<(Block, Vec<Receipt>)> {
        let proposal = self.generate_proposal(txs).c(d!())?;

        let executor_ctx = ExecutorContext::from(&proposal);
        let mut evm_exec_backend = EvmExecBackend::from_root(
            self.prev_state_root,
            &self.trie,
            &self.storage,
            executor_ctx,
        )
        .c(d!())?;
        let (exec_resp, mut system_txs) =
            Executor.exec(&mut evm_exec_backend, txs, system_contracts)?;

        self.mempool.tx_cleanup(txs);

        let block = Block::new(proposal, &exec_resp);
        let mut tmp = txs.to_vec();
        if !system_txs.is_empty() {
            tmp.append(&mut system_txs);
        }
        let receipts = generate_receipts(
            self.block_number,
            block.hash(),
            block.header.state_root,
            &tmp,
            &exec_resp,
        );

        Ok((block, receipts))
    }

    pub fn generate_proposal(&self, txs: &[SignedTransaction]) -> Result<Proposal> {
        let p = Proposal {
            prev_hash: self.prev_block_hash,
            proposer: self.proposer,
            transactions_root: trie_root_txs(txs),
            timestamp: self.block_timestamp,
            number: self.block_number,
            gas_limit: MAX_BLOCK_GAS_LIMIT.into(),
            extra_data: Default::default(),
            mixed_hash: None,
            base_fee_per_gas: BASE_FEE_PER_GAS.into(),
            chain_id: self.chain_id,
            tx_hashes: txs.iter().map(|tx| tx.transaction.hash).collect(),
        };

        Ok(p)
    }

    pub fn verify_block(&self, fb: &FatBlock) -> Result<()> {
        self.verify_refblock(fb.into())
    }

    pub fn verify_refblock(&self, fb: FatBlockRef) -> Result<()> {
        self.verify_proposal(&fb.block.into(), fb.txs).c(d!())
    }

    pub fn verify_proposal(
        &self,
        p: &Proposal,
        txs: &[SignedTransaction],
    ) -> Result<()> {
        if p.number < 1 {
            return Err(eg!());
        }

        if self.chain_id != p.chain_id {
            return Err(eg!());
        }

        if ts!() < p.timestamp {
            return Err(eg!());
        }

        let prev_header = self
            .storage
            .get_block_header(p.number - 1)
            .c(d!())?
            .c(d!())?;

        if p.prev_hash != prev_header.hash() {
            return Err(eg!());
        }

        let txs_root = trie_root_indexed(&p.tx_hashes.iter().collect::<Vec<_>>());
        if txs_root != p.transactions_root {
            return Err(eg!());
        }

        if txs.len() != p.tx_hashes.len() {
            return Err(eg!());
        }

        (0..txs.len())
            .into_par_iter()
            .map(|i| (&txs[i], &p.tx_hashes[i]))
            .try_for_each(|(tx, hash_in_block)| {
                if &tx.transaction.hash != hash_in_block {
                    return Err(eg!("Hash mismatch"));
                }
                self.mempool.tx_pre_check(tx, false).c(d!())
            })
    }
}

fn generate_receipts(
    block_number: u64,
    block_hash: Hash,
    state_root: MerkleRoot,
    txs: &[SignedTransaction],
    resp: &ExecResp,
) -> Vec<Receipt> {
    let mut log_index = 0;
    txs.iter()
        .enumerate()
        .zip(resp.txs_resp.iter())
        .map(|((idx, tx), res)| {
            let receipt = Receipt {
                tx_hash: tx.transaction.hash,
                block_number,
                block_hash,
                tx_index: idx as u32,
                state_root,
                used_gas: U256::from(res.gas_used),
                logs_bloom: logs_bloom(res.logs.iter()),
                logs: res.logs.clone(),
                log_index,
                code_address: res.code_address,
                sender: tx.sender,
                ret: res.exit_reason.clone(),
                removed: res.removed,
            };
            log_index += res.logs.len() as u32;
            receipt
        })
        .collect()
}
