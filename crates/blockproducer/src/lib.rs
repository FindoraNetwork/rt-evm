use rt_evm_executor::{
    logs_bloom, trie_root, RTEvmExecutor as Executor,
    RTEvmExecutorAdapter as EvmExecBackend,
};
use rt_evm_mempool::Mempool;
use rt_evm_model::{
    traits::Executor as _,
    types::{
        Block, ExecResp, ExecutorContext, Hash, MerkleRoot, Proposal, Receipt,
        SignedTransaction, BASE_FEE_PER_GAS, H160, MAX_BLOCK_GAS_LIMIT, U256,
    },
};
use rt_evm_storage::{MptStore, Storage};
use ruc::*;

pub struct BlockProducer<'a> {
    pub proposer: H160,

    // the state hash of the previous block
    pub prev_hash: MerkleRoot,

    // the height of the proposing block
    pub block_number: u64,

    // the timestamp of the proposing block
    pub block_timestamp: u64,

    pub chain_id: u64,

    pub mempool: &'a Mempool,
    pub trie: &'a MptStore,
    pub storage: &'a Storage,
}

impl<'a> BlockProducer<'a> {
    pub fn new_block(&self, txs: &[SignedTransaction]) -> Result<(Block, Vec<Receipt>)> {
        let proposal = self.new_proposal(txs);

        let executor_ctx = ExecutorContext::from(&proposal);
        let mut evm_exec_backend = EvmExecBackend::from_root(
            self.prev_hash,
            self.trie,
            self.storage,
            executor_ctx,
        )
        .c(d!())?;
        let exec_resp = Executor.exec(&mut evm_exec_backend, txs);

        self.mempool.tx_cleanup(&proposal.tx_hashes);

        let block = Block::new(proposal, &exec_resp);
        let receipts = generate_receipts(
            self.block_number,
            block.hash(),
            block.header.state_root,
            txs,
            &exec_resp,
        );

        Ok((block, receipts))
    }

    fn new_proposal(&self, txs: &[SignedTransaction]) -> Proposal {
        let tx_hashes_indexed = txs
            .iter()
            .map(|tx| tx.transaction.hash)
            .enumerate()
            .map(|(i, h)| (u32::to_be_bytes(i as u32), h))
            .collect::<Vec<_>>();
        let transactions_root = trie_root(tx_hashes_indexed);

        Proposal {
            prev_hash: self.prev_hash,
            proposer: self.proposer,
            transactions_root,
            timestamp: self.block_timestamp,
            number: self.block_number,
            gas_limit: MAX_BLOCK_GAS_LIMIT.into(),
            extra_data: Default::default(),
            base_fee_per_gas: BASE_FEE_PER_GAS.into(),
            chain_id: self.chain_id,
            tx_hashes: txs.iter().map(|tx| tx.transaction.hash).collect(),
        }
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
