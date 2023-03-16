use evm::backend::{Apply, Basic};
use rt_evm_model::{
    codec::ProtocolCodec,
    traits::{ApplyBackend, Backend, BlockStorage, ExecutorAdapter, TxStorage},
    types::{
        Account, ExecutorContext, Hasher, Log, MerkleRoot, Proposal, GB, H160, H256, MB,
        NIL_DATA, RLP_NULL, U256,
    },
};
use rt_evm_storage::{
    trie_db::{MptOnce, MptStore},
    FunStorage,
};
use ruc::*;
use std::mem;

const GET_BLOCK_HASH_NUMBER_RANGE: u64 = 256;

type WorldStateMpt<'a> = MptOnce<'a>;
type GlobalState<'a> = WorldStateMpt<'a>;

pub struct RTEvmExecutorAdapter<'a> {
    state: GlobalState<'a>,
    trie: &'a MptStore,
    storage: &'a FunStorage,
    exec_ctx: ExecutorContext,
}

impl<'a> ExecutorAdapter for RTEvmExecutorAdapter<'a> {
    fn get_ctx(&self) -> ExecutorContext {
        self.exec_ctx.clone()
    }

    fn set_origin(&mut self, origin: H160) {
        self.exec_ctx.origin = origin;
    }

    fn set_gas_price(&mut self, gas_price: U256) {
        self.exec_ctx.gas_price = gas_price;
    }

    fn get_logs(&mut self) -> Vec<Log> {
        mem::take(&mut self.exec_ctx.logs)
    }

    fn commit(&mut self) -> MerkleRoot {
        self.state.commit()
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.state.get(key).ok().flatten()
    }

    fn get_account(&self, address: &H160) -> Account {
        if let Ok(Some(raw)) = self.state.get(address.as_bytes()) {
            return pnk!(Account::decode(raw));
        }

        Account {
            nonce: U256::zero(),
            balance: U256::zero(),
            storage_root: RLP_NULL,
            code_hash: NIL_DATA,
        }
    }

    fn save_account(&mut self, address: &H160, account: &Account) {
        let acc = pnk!(account.encode());
        pnk!(self.state.insert(address.as_bytes(), &acc))
    }
}

impl<'a> Backend for RTEvmExecutorAdapter<'a> {
    fn gas_price(&self) -> U256 {
        self.exec_ctx.gas_price
    }

    fn origin(&self) -> H160 {
        self.exec_ctx.origin
    }

    fn block_number(&self) -> U256 {
        self.exec_ctx.block_number
    }

    fn block_hash(&self, number: U256) -> H256 {
        let current_number = self.block_number();
        if number >= current_number {
            return H256::default();
        }

        if (current_number - number) > U256::from(GET_BLOCK_HASH_NUMBER_RANGE) {
            return H256::default();
        }

        let number = number.as_u64();
        let res = self.storage.get_block(number).unwrap();

        res.map(|b| Proposal::from(&b).hash()).unwrap_or_default()
    }

    fn block_coinbase(&self) -> H160 {
        self.exec_ctx.block_coinbase
    }

    fn block_timestamp(&self) -> U256 {
        self.exec_ctx.block_timestamp
    }

    fn block_difficulty(&self) -> U256 {
        self.exec_ctx.difficulty
    }

    fn block_gas_limit(&self) -> U256 {
        self.exec_ctx.block_gas_limit
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.exec_ctx.block_base_fee_per_gas
    }

    fn chain_id(&self) -> U256 {
        self.exec_ctx.chain_id
    }

    fn exists(&self, address: H160) -> bool {
        self.state.contains(address.as_bytes()).unwrap_or_default()
    }

    fn basic(&self, address: H160) -> Basic {
        self.state
            .get(address.as_bytes())
            .map(|raw| {
                if raw.is_none() {
                    return Basic::default();
                }
                Account::decode(raw.unwrap()).map_or_else(
                    |_| Default::default(),
                    |account| Basic {
                        balance: account.balance,
                        nonce: account.nonce,
                    },
                )
            })
            .unwrap_or_default()
    }

    fn code(&self, address: H160) -> Vec<u8> {
        let code_hash = if let Some(bytes) = self.state.get(address.as_bytes()).unwrap()
        {
            Account::decode(bytes).unwrap().code_hash
        } else {
            return Vec::new();
        };

        if code_hash == NIL_DATA {
            return Vec::new();
        }

        let res = self.storage.get_code_by_hash(&code_hash).unwrap();

        res.unwrap_or_default()
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        if let Ok(raw) = self.state.get(address.as_bytes()) {
            if raw.is_none() {
                return H256::default();
            }

            Account::decode(raw.unwrap())
                .and_then(|account| {
                    let storage_root = account.storage_root;
                    if storage_root == RLP_NULL {
                        Ok(H256::default())
                    } else {
                        self.trie
                            .trie_restore(address.as_bytes(), storage_root)
                            .map(|trie| match trie.get(index.as_bytes()) {
                                Ok(Some(res)) => H256::from_slice(res.as_ref()),
                                _ => H256::default(),
                            })
                    }
                })
                .unwrap_or_default()
        } else {
            H256::default()
        }
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        // fixme
        Some(self.storage(address, index))
    }
}

impl<'a> ApplyBackend for RTEvmExecutorAdapter<'a> {
    fn apply<A, I, L>(&mut self, values: A, logs: L, delete_empty: bool)
    where
        A: IntoIterator<Item = Apply<I>>,
        I: IntoIterator<Item = (H256, H256)>,
        L: IntoIterator<Item = Log>,
    {
        for apply in values.into_iter() {
            match apply {
                Apply::Modify {
                    address,
                    basic,
                    code,
                    storage,
                    reset_storage,
                } => {
                    let is_empty =
                        self.apply(address, basic, code, storage, reset_storage);
                    if is_empty && delete_empty {
                        self.state.remove(address.as_bytes()).unwrap();
                    }
                }
                Apply::Delete { address } => {
                    let _ = self.state.remove(address.as_bytes());
                }
            }
        }

        self.exec_ctx.logs = logs.into_iter().collect::<Vec<_>>();
    }
}

impl<'a> RTEvmExecutorAdapter<'a> {
    pub const WORLD_STATE_META_KEY: [u8; 1] = [0];

    const WORLD_STATE_CACHE_SIZE: usize = 4 * GB;
    const ACCOUNT_STATE_CACHE_SIZE: usize = 32 * MB;

    pub fn new(
        trie: &'a MptStore,
        storage: &'a FunStorage,
        exec_ctx: ExecutorContext,
    ) -> Result<Self> {
        let state = trie
            .trie_create(&Self::WORLD_STATE_META_KEY, Self::WORLD_STATE_CACHE_SIZE)
            .c(d!())?;
        Ok(RTEvmExecutorAdapter {
            state,
            trie,
            storage,
            exec_ctx,
        })
    }

    pub fn from_root(
        state_root: MerkleRoot,
        trie: &'a MptStore,
        storage: &'a FunStorage,
        exec_ctx: ExecutorContext,
    ) -> Result<Self> {
        let state = trie
            .trie_restore(&Self::WORLD_STATE_META_KEY, state_root)
            .c(d!())?;

        Ok(RTEvmExecutorAdapter {
            state,
            trie,
            storage,
            exec_ctx,
        })
    }

    pub fn apply<I: IntoIterator<Item = (H256, H256)>>(
        &mut self,
        address: H160,
        basic: Basic,
        code: Option<Vec<u8>>,
        storage: I,
        reset_storage: bool,
    ) -> bool {
        let old_account = match self.state.get(address.as_bytes()) {
            Ok(Some(raw)) => Account::decode(raw).unwrap(),
            _ => Account {
                nonce: U256::zero(),
                balance: U256::zero(),
                storage_root: RLP_NULL,
                code_hash: NIL_DATA,
            },
        };

        let storage_root = if reset_storage {
            RLP_NULL
        } else {
            old_account.storage_root
        };

        let mut storage_trie = if storage_root == RLP_NULL {
            pnk!(
                self.trie
                    .trie_create(address.as_bytes(), Self::ACCOUNT_STATE_CACHE_SIZE)
            )
        } else {
            pnk!(
                self.trie
                    .trie_restore(address.as_bytes(), old_account.storage_root)
            )
        };

        storage.into_iter().for_each(|(k, v)| {
            let _ = storage_trie.insert(k.as_bytes(), v.as_bytes());
        });

        let mut new_account = Account {
            nonce: basic.nonce,
            balance: basic.balance,
            code_hash: old_account.code_hash,
            storage_root: storage_trie.commit(),
        };

        if let Some(c) = code {
            let new_code_hash = Hasher::digest(&c);
            if new_code_hash != old_account.code_hash {
                self.storage
                    .insert_code(address.into(), new_code_hash, c)
                    .unwrap();
                new_account.code_hash = new_code_hash;
            }
        }

        let bytes = new_account.encode().unwrap();

        {
            self.state
                .insert(address.as_bytes(), bytes.as_ref())
                .unwrap();
        }

        new_account.balance == U256::zero()
            && new_account.nonce == U256::zero()
            && new_account.code_hash.is_zero()
    }
}
