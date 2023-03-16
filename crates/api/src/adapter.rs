use rt_evm_executor::{RTEvmExecutor, RTEvmExecutorAdapter};
use rt_evm_mempool::Mempool;
use rt_evm_model::{
    async_trait,
    codec::ProtocolCodec,
    traits::{APIAdapter, BlockStorage, Executor, ExecutorAdapter, TxStorage},
    types::{
        Account, BigEndianHash, Block, BlockNumber, ExecutorContext, Hash, Header,
        Proposal, Receipt, SignedTransaction, TxResp, H160, MAX_BLOCK_GAS_LIMIT,
        NIL_DATA, RLP_NULL, U256,
    },
};
use rt_evm_storage::{FunStorage, MptStore};
use ruc::*;
use std::sync::Arc;

pub struct DefaultAPIAdapter {
    mempool: Arc<Mempool>,
    trie: Arc<MptStore>,
    storage: Arc<FunStorage>,
}

impl DefaultAPIAdapter {
    pub fn new(
        mempool: Arc<Mempool>,
        trie: Arc<MptStore>,
        storage: Arc<FunStorage>,
    ) -> Self {
        Self {
            mempool,
            trie,
            storage,
        }
    }

    pub async fn evm_backend(
        &self,
        number: Option<BlockNumber>,
    ) -> Result<RTEvmExecutorAdapter> {
        let block = self
            .get_block_by_number(number)
            .await?
            .c(d!("Cannot get {:?} block", number))?;
        let state_root = block.header.state_root;
        let proposal = Proposal::from(&block);

        RTEvmExecutorAdapter::from_root(
            state_root,
            &self.trie,
            &self.storage,
            ExecutorContext::from(&proposal),
        )
    }
}

#[async_trait]
impl APIAdapter for DefaultAPIAdapter {
    async fn insert_signed_tx(&self, signed_tx: SignedTransaction) -> Result<()> {
        self.mempool.tx_insert(signed_tx).c(d!())
    }

    async fn get_block_by_number(&self, height: Option<u64>) -> Result<Option<Block>> {
        match height {
            Some(number) => self.storage.get_block(number),
            None => self.storage.get_latest_block().map(Option::Some),
        }
    }

    async fn get_block_by_hash(&self, hash: Hash) -> Result<Option<Block>> {
        self.storage.get_block_by_hash(&hash)
    }

    async fn get_block_header_by_number(
        &self,
        number: Option<u64>,
    ) -> Result<Option<Header>> {
        match number {
            Some(num) => self.storage.get_block_header(num),
            None => self.storage.get_latest_block_header().map(Option::Some),
        }
    }

    async fn get_receipt_by_tx_hash(&self, tx_hash: Hash) -> Result<Option<Receipt>> {
        self.storage.get_receipt_by_hash(&tx_hash)
    }

    async fn get_receipts_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<Receipt>>> {
        self.storage.get_receipts(block_number, tx_hashes)
    }

    async fn get_transaction_by_hash(
        &self,
        tx_hash: Hash,
    ) -> Result<Option<SignedTransaction>> {
        self.storage.get_transaction_by_hash(&tx_hash)
    }

    async fn get_transactions_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<SignedTransaction>>> {
        self.storage.get_transactions(block_number, tx_hashes)
    }

    async fn get_account(
        &self,
        address: H160,
        number: Option<BlockNumber>,
    ) -> Result<Account> {
        match self.evm_backend(number).await?.get(address.as_bytes()) {
            Some(bytes) => Account::decode(bytes),
            None => Ok(Account {
                nonce: U256::zero(),
                balance: U256::zero(),
                storage_root: RLP_NULL,
                code_hash: NIL_DATA,
            }),
        }
    }

    async fn get_pending_tx_count(&self, address: H160) -> Result<U256> {
        Ok(self.mempool.tx_pending_cnt(Some(address)).into())
    }

    async fn evm_call(
        &self,
        from: Option<H160>,
        to: Option<H160>,
        gas_price: Option<U256>,
        gas_limit: Option<U256>,
        value: U256,
        data: Vec<u8>,
        state_root: Hash,
        mock_header: Proposal,
    ) -> Result<TxResp> {
        let mut exec_ctx = ExecutorContext::from(&mock_header);
        exec_ctx.origin = from.unwrap_or_default();
        exec_ctx.gas_price = gas_price.unwrap_or_else(U256::one);

        let backend = RTEvmExecutorAdapter::from_root(
            state_root,
            &self.trie,
            &self.storage,
            exec_ctx,
        )?;
        let gas_limit = gas_limit
            .map(|gas| gas.as_u64())
            .unwrap_or(MAX_BLOCK_GAS_LIMIT);

        Ok(RTEvmExecutor::default().call(&backend, gas_limit, from, to, value, data))
    }

    async fn get_code_by_hash(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        self.storage.get_code_by_hash(hash)
    }

    async fn get_storage_at(
        &self,
        address: H160,
        position: U256,
        state_root: Hash,
    ) -> Result<Vec<u8>> {
        let state_trie_tree = self
            .trie
            .trie_restore(&RTEvmExecutorAdapter::WORLD_STATE_META_KEY, state_root)
            .c(d!())?;

        let raw_account = state_trie_tree
            .get(address.as_bytes())
            .c(d!("Can't find this address"))?
            .c(d!("Can't find this address"))?;

        let account = Account::decode(raw_account).unwrap();

        let storage_trie_tree = self
            .trie
            .trie_restore(address.as_bytes(), account.storage_root)
            .c(d!())?;

        let hash: Hash = BigEndianHash::from_uint(&position);
        storage_trie_tree
            .get(hash.as_bytes())
            .c(d!("Can't find this position"))?
            .c(d!("Can't find this position"))
    }
}
