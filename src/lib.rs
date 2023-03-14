//!
//! A pre-built simple entry to the `rt-evm` development framework.
//!

pub use rt_evm_api as api;
pub use rt_evm_blockproducer as blockproducer;
pub use rt_evm_executor as executor;
pub use rt_evm_mempool as mempool;
pub use rt_evm_model as model;
pub use rt_evm_storage as storage;

use blockproducer::BlockProducer;
use executor::RTEvmExecutorAdapter;
use mempool::Mempool;
use model::{
    traits::BlockStorage as _,
    types::{Basic, Block, H160, U256},
};
use once_cell::sync::Lazy;
use ruc::*;
use std::{fs, mem::size_of, path::PathBuf, sync::Arc};
use storage::{MptStore, Storage};

static META_PATH: Lazy<MetaPath> = Lazy::new(|| {
    let mut trie = vsdb::vsdb_get_base_dir();
    let mut storage = trie.clone();
    let mut chain_id = trie.clone();

    trie.push("__evm_runtime_trie.meta");
    storage.push("__evm_runtime_storage.meta");
    chain_id.push("__evm_runtime_chain_id.meta");

    MetaPath {
        chain_id,
        trie,
        storage,
    }
});

pub struct EvmRuntime {
    chain_id: u64,

    // create a new instance every time
    mempool: Arc<Mempool>,

    trie: Arc<MptStore>,
    storage: Arc<Storage>,
}

impl EvmRuntime {
    fn new(chain_id: u64, t: MptStore, s: Storage) -> Self {
        Self {
            chain_id,
            trie: Arc::new(t),
            storage: Arc::new(s),
            mempool: Arc::new(Mempool::default()),
        }
    }

    pub fn create(
        chain_id: u64,
        token_distributions: &[TokenDistributon],
    ) -> Result<Self> {
        let r = Self::new(chain_id, MptStore::new(), Storage::default());

        {
            let mut exector_adapter =
                RTEvmExecutorAdapter::new(&r.trie, &r.storage, Default::default())
                    .c(d!())?;

            token_distributions
                .iter()
                .fold(map! {}, |mut acc, td| {
                    let hdr = acc.entry(td.address).or_insert(*td);
                    if td.amount != hdr.amount {
                        hdr.amount = hdr.amount.saturating_add(td.amount);
                    }
                    acc
                })
                .into_values()
                .for_each(|td| {
                    exector_adapter.apply(td.address, td.basic(), None, vec![], true);
                });
        }

        // set a genesis block first
        r.storage.set_block(Block::genesis(chain_id)).c(d!())?;

        // Only need to write once time !
        fs::write(META_PATH.trie.as_path(), u64::to_be_bytes(r.chain_id)).c(d!())?;

        // Only need to write once time !
        bcs::to_bytes(&*r.trie)
            .c(d!())
            .and_then(|bytes| fs::write(META_PATH.trie.as_path(), bytes).c(d!()))?;

        // Only need to write once time !
        bcs::to_bytes(&*r.storage)
            .c(d!())
            .and_then(|bytes| fs::write(META_PATH.storage.as_path(), bytes).c(d!()))?;

        Ok(r)
    }

    pub fn restore() -> Result<Self> {
        let chain_id = fs::read(META_PATH.chain_id.as_path())
            .c(d!())
            .and_then(|bytes| {
                <[u8; size_of::<u64>()]>::try_from(bytes)
                    .map_err(|_| eg!("invalid length"))
            })
            .map(u64::from_be_bytes)?;
        let trie = fs::read(META_PATH.trie.as_path())
            .c(d!())
            .and_then(|m| bcs::from_bytes::<MptStore>(&m).c(d!()))?;

        let storage = fs::read(META_PATH.storage.as_path())
            .c(d!())
            .and_then(|m| bcs::from_bytes::<Storage>(&m).c(d!()))?;

        Ok(Self::new(chain_id, trie, storage))
    }

    pub fn restore_or_create(
        chain_id: u64,
        token_distributions: &[TokenDistributon],
    ) -> Result<Self> {
        Self::restore()
            .c(d!())
            .or_else(|_| Self::create(chain_id, token_distributions).c(d!()))
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn mempool_handler(&self) -> &Mempool {
        &self.mempool
    }

    pub fn trie_handler(&self) -> &MptStore {
        &self.trie
    }

    pub fn storage_handler(&self) -> &Storage {
        &self.storage
    }

    pub fn copy_mempool_handler(&self) -> Arc<Mempool> {
        Arc::clone(&self.mempool)
    }

    pub fn copy_trie_handler(&self) -> Arc<MptStore> {
        Arc::clone(&self.trie)
    }

    pub fn copy_storage_handler(&self) -> Arc<Storage> {
        Arc::clone(&self.storage)
    }

    pub fn generate_blockproducer(&self, proposer: H160) -> Result<BlockProducer> {
        let header = self.storage.get_latest_block_header().c(d!())?;
        Ok(BlockProducer {
            proposer,
            prev_block_hash: header.hash(),
            prev_state_root: header.state_root,
            block_number: header.number + 1,
            block_timestamp: ts!(),
            chain_id: self.chain_id,
            mempool: self.copy_mempool_handler(),
            trie: self.copy_trie_handler(),
            storage: self.copy_storage_handler(),
        })
    }
}

struct MetaPath {
    chain_id: PathBuf,
    trie: PathBuf,
    storage: PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub struct TokenDistributon {
    address: H160,
    amount: U256,
}

impl TokenDistributon {
    pub fn new(address: H160, amount: U256) -> Self {
        Self { address, amount }
    }

    fn basic(&self) -> Basic {
        Basic {
            balance: self.amount,
            nonce: Default::default(),
        }
    }
}
