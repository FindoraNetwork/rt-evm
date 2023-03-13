//!
//! A pre-built simple entry to the `rt-evm` development framework.
//!

pub use rt_evm_api as api;
pub use rt_evm_blockproducer as blockproducer;
pub use rt_evm_executor as executor;
pub use rt_evm_mempool as mempool;
pub use rt_evm_model as model;
pub use rt_evm_storage as storage;

use executor::RTEvmExecutorAdapter;
use model::types::{Basic, H160, U256};
use once_cell::sync::Lazy;
use ruc::*;
use std::{fs, path::PathBuf, sync::Arc};
use storage::{MptStore, Storage};

static META_PATH: Lazy<MetaPath> = Lazy::new(|| {
    let mut base_t = vsdb::vsdb_get_base_dir();
    let mut base_s = base_t.clone();

    base_t.push("evm_runtime_trie.meta");
    base_s.push("evm_runtime_storage.meta");

    MetaPath {
        trie: base_t,
        storage: base_s,
    }
});

pub struct EvmRuntime {
    trie: Arc<MptStore>,
    storage: Arc<Storage>,
}

impl EvmRuntime {
    fn new(t: MptStore, s: Storage) -> Self {
        Self {
            trie: Arc::new(t),
            storage: Arc::new(s),
        }
    }

    pub fn create(token_distributions: &[TokenDistributon]) -> Result<Self> {
        let r = Self {
            trie: Arc::new(MptStore::new()),
            storage: Arc::new(Storage::default()),
        };

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

        // Only need to write once time !
        bcs::to_bytes(&*r.trie)
            .c(d!())
            .and_then(|bytes| fs::write(META_PATH.trie.as_path(), bytes).c(d!()))?;

        bcs::to_bytes(&*r.storage)
            .c(d!())
            .and_then(|bytes| fs::write(META_PATH.storage.as_path(), bytes).c(d!()))?;

        Ok(r)
    }

    pub fn restore() -> Result<Self> {
        let trie = fs::read_to_string(META_PATH.trie.as_path())
            .c(d!())
            .and_then(|m| bcs::from_bytes::<MptStore>(m.as_bytes()).c(d!()))?;

        let storage = fs::read_to_string(META_PATH.storage.as_path())
            .c(d!())
            .and_then(|m| bcs::from_bytes::<Storage>(m.as_bytes()).c(d!()))?;

        Ok(Self::new(trie, storage))
    }

    pub fn restore_or_create(token_distributions: &[TokenDistributon]) -> Result<Self> {
        Self::restore()
            .c(d!())
            .or_else(|_| Self::create(token_distributions).c(d!()))
    }

    pub fn get_trie_handler(&self) -> Arc<MptStore> {
        Arc::clone(&self.trie)
    }

    pub fn get_storage_handler(&self) -> Arc<Storage> {
        Arc::clone(&self.storage)
    }
}

struct MetaPath {
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
