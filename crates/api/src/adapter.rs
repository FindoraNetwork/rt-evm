use rt_evm_executor::{RTEvmExecutor, RTEvmExecutorAdapter};
use rt_evm_mempool::Mempool;
use rt_evm_model::{
    codec::ProtocolCodec,
    traits::{
        APIAdapter, BlockStorage, Executor, ExecutorAdapter, TxStorage, BALANCE_SLOT,
        CONSTANT_ADDR,
    },
    types::{
        Account, BigEndianHash, Block, BlockNumber, ExecutorContext, Hash, Hasher,
        Header, Proposal, Receipt, SignedTransaction, TxResp, H160, H256,
        MAX_BLOCK_GAS_LIMIT, MIN_GAS_PRICE, NIL_HASH, U256, WORLD_STATE_META_KEY,
    },
};
use rt_evm_storage::{
    ethabi::{encode, Token},
    MptStore, Storage,
};
use ruc::*;
use std::sync::Arc;

pub struct DefaultAPIAdapter {
    mempool: Arc<Mempool>,
    trie_db: Arc<MptStore>,
    storage: Arc<Storage>,
}

impl DefaultAPIAdapter {
    pub fn new(
        mempool: Arc<Mempool>,
        trie_db: Arc<MptStore>,
        storage: Arc<Storage>,
    ) -> Self {
        Self {
            mempool,
            trie_db,
            storage,
        }
    }

    pub fn evm_backend(
        &self,
        number: Option<BlockNumber>,
    ) -> Result<RTEvmExecutorAdapter> {
        let block = self
            .get_block_by_number(number)
            .c(d!())?
            .c(d!("Cannot get {:?} block", number))?;
        let state_root = block.header.state_root;
        let proposal = Proposal::from(&block);

        RTEvmExecutorAdapter::from_root(
            state_root,
            &self.trie_db,
            &self.storage,
            ExecutorContext::from(&proposal),
        )
    }
}

impl APIAdapter for DefaultAPIAdapter {
    fn insert_signed_tx(&self, signed_tx: SignedTransaction) -> Result<()> {
        self.mempool.tx_insert(signed_tx, true).c(d!())
    }

    fn get_block_by_number(&self, height: Option<u64>) -> Result<Option<Block>> {
        match height {
            Some(number) => self.storage.get_block(number),
            None => self.storage.get_latest_block().map(Option::Some),
        }
    }

    fn get_block_by_hash(&self, hash: Hash) -> Result<Option<Block>> {
        self.storage.get_block_by_hash(&hash)
    }

    fn get_block_header_by_number(&self, number: Option<u64>) -> Result<Option<Header>> {
        match number {
            Some(num) => self.storage.get_block_header(num),
            None => self.storage.get_latest_block_header().map(Option::Some),
        }
    }

    fn get_receipt_by_tx_hash(&self, tx_hash: Hash) -> Result<Option<Receipt>> {
        self.storage.get_receipt_by_hash(&tx_hash)
    }

    fn get_receipts_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<Receipt>>> {
        self.storage.get_receipts(block_number, tx_hashes)
    }

    fn get_tx_by_hash(&self, tx_hash: Hash) -> Result<Option<SignedTransaction>> {
        self.storage.get_tx_by_hash(&tx_hash)
    }

    fn get_txs_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<SignedTransaction>>> {
        self.storage.get_txs(block_number, tx_hashes)
    }

    fn get_account(
        &self,
        address: H160,
        number: Option<BlockNumber>,
    ) -> Result<Account> {
        let state = self.evm_backend(number).c(d!())?;
        let mut account = match state.get(address.as_bytes()) {
            Some(bytes) => Account::decode(bytes),
            None => Ok(Account {
                nonce: U256::zero(),
                balance: U256::zero(),
                storage_root: NIL_HASH,
                code_hash: NIL_HASH,
            }),
        }?;

        if let Some(addr) = CONSTANT_ADDR.get() {
            account.balance = U256::zero();
            let storage_root = match state.get(addr.as_bytes()) {
                Some(bytes) => Account::decode(bytes),
                None => Ok(Account {
                    nonce: U256::zero(),
                    balance: U256::zero(),
                    storage_root: NIL_HASH,
                    code_hash: NIL_HASH,
                }),
            }?
            .storage_root;

            if storage_root != NIL_HASH {
                if let Ok(storage_trie_tree) = self
                    .trie_db
                    .trie_restore(addr.as_bytes(), storage_root.into())
                {
                    let idx = Hasher::digest(&encode(&[
                        Token::Address(address),
                        Token::Uint(*BALANCE_SLOT.get().c(d!())?),
                    ]));
                    storage_trie_tree.get(idx.as_bytes())?.map(|balance| {
                        account.balance = H256::from_slice(&balance).into_uint()
                    });
                };
            }
        }
        Ok(account)
    }

    fn get_pending_tx_count(&self, address: H160) -> Result<U256> {
        Ok(self.mempool.tx_pending_cnt(Some(address)).into())
    }

    fn evm_call(
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
        exec_ctx.gas_price = gas_price.unwrap_or_else(|| U256::from(MIN_GAS_PRICE));

        let backend = RTEvmExecutorAdapter::from_root(
            state_root,
            &self.trie_db,
            &self.storage,
            exec_ctx,
        )?;
        let gas_limit = gas_limit
            .map(|gas| gas.as_u64())
            .unwrap_or(MAX_BLOCK_GAS_LIMIT);

        Ok(RTEvmExecutor.call(&backend, gas_limit, from, to, value, data))
    }

    fn get_code_by_hash(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        self.storage.get_code_by_hash(hash)
    }

    fn get_storage_at(
        &self,
        address: H160,
        position: U256,
        state_root: Hash,
    ) -> Result<Vec<u8>> {
        let state_trie_tree = self
            .trie_db
            .trie_restore(&WORLD_STATE_META_KEY, state_root.into())
            .c(d!())?;

        let raw_account = state_trie_tree
            .get(address.as_bytes())
            .c(d!("Can't find this address"))?
            .c(d!("Can't find this address"))?;

        let account = Account::decode(raw_account).unwrap();

        let storage_trie_tree = self
            .trie_db
            .trie_restore(address.as_bytes(), account.storage_root.into())
            .c(d!())?;

        let hash: Hash = BigEndianHash::from_uint(&position);
        storage_trie_tree
            .get(hash.as_bytes())
            .c(d!("Can't find this position"))?
            .c(d!("Can't find this position"))
    }
}
