pub mod trie_db;

pub use trie_db::MptStore;
pub use FunStorage as Storage;

use moka::sync::Cache as Lru;
use parking_lot::RwLock;
use rt_evm_model::{
    traits::{BlockStorage, TxStorage},
    types::{Block, BlockNumber, Hash, Header, Receipt, SignedTransaction, H256},
};
use ruc::*;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use vsdb::{MapxOrd, MapxRaw};

const BATCH_LIMIT: usize = 1000;

#[derive(Debug, Serialize, Deserialize)]
pub struct FunStorage {
    db: DB,
    #[serde(skip)]
    cache: Cache,
}

const DEFAULT_CACHE_SIZE: u64 = 100_0000;

impl FunStorage {
    pub fn new(cache_size: u64) -> Self {
        Self {
            db: DB::new(),
            cache: Cache::new(cache_size),
        }
    }
}

impl Default for FunStorage {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_SIZE)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DB {
    blocks: MapxOrd<u64, Block>,
    block_numbers: MapxOrd<Hash, u64>,
    headers: MapxOrd<u64, Header>,

    transactions: MapxOrd<Hash, (BlockNumber, SignedTransaction)>,

    codes: MapxRaw,
    codes_addr_to_hash: MapxRaw,

    receipts: MapxOrd<Hash, Receipt>,
}

impl DB {
    fn new() -> Self {
        Self {
            blocks: MapxOrd::new(),
            block_numbers: MapxOrd::new(),
            headers: MapxOrd::new(),

            transactions: MapxOrd::new(),

            codes: MapxRaw::new(),
            codes_addr_to_hash: MapxRaw::new(),

            receipts: MapxOrd::new(),
        }
    }

    // safe in the context of EVM
    fn shadow(&self) -> Self {
        unsafe {
            Self {
                blocks: self.blocks.shadow(),
                block_numbers: self.block_numbers.shadow(),
                headers: self.headers.shadow(),
                transactions: self.transactions.shadow(),
                codes: self.codes.shadow(),
                codes_addr_to_hash: self.codes_addr_to_hash.shadow(),
                receipts: self.receipts.shadow(),
            }
        }
    }
}

#[macro_export(local_inner_macros)]
macro_rules! gen_lru {
    ($size: expr) => {
        Lru::builder()
            .max_capacity($size)
            .time_to_idle(Duration::from_secs(600)) // 10 minutes
            .build()
    };
}

#[derive(Debug, Clone)]
struct Cache {
    blocks: Lru<u64, Block>,
    block_numbers: Lru<Hash, u64>,
    headers: Lru<u64, Header>,

    transactions: Lru<Hash, (BlockNumber, SignedTransaction)>,

    codes: Lru<Hash, Vec<u8>>,
    codes_addr_to_hash: Lru<H256, Hash>,

    receipts: Lru<Hash, Receipt>,

    latest_block: Arc<RwLock<Option<Block>>>,
}

impl Cache {
    fn new(size: u64) -> Self {
        Self {
            blocks: gen_lru!(size),
            block_numbers: gen_lru!(size),
            headers: gen_lru!(size),
            transactions: gen_lru!(size),
            codes: gen_lru!(size),
            codes_addr_to_hash: gen_lru!(size),
            receipts: gen_lru!(size),
            latest_block: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_SIZE)
    }
}

impl BlockStorage for FunStorage {
    fn set_block(&self, block: Block) -> Result<()> {
        let mut db = self.db.shadow();

        let bh = block.hash();
        let header = block.header.clone();
        let height = header.number;

        db.block_numbers.insert(&bh, &height);
        db.headers.insert(&height, &header);
        db.blocks.insert(&height, &block);

        self.cache.block_numbers.insert(bh, height);
        self.cache.headers.insert(height, header);
        self.cache.blocks.insert(height, block.clone());

        self.set_latest_block(block).c(d!())
    }

    fn get_block(&self, height: u64) -> Result<Option<Block>> {
        Ok(self
            .cache
            .blocks
            .get(&height)
            .or_else(|| self.db.blocks.get(&height)))
    }

    fn get_block_header(&self, height: u64) -> Result<Option<Header>> {
        Ok(self
            .cache
            .headers
            .get(&height)
            .or_else(|| self.db.headers.get(&height)))
    }

    fn get_latest_block(&self) -> Result<Block> {
        Ok(self
            .cache
            .latest_block
            .read()
            .clone()
            .or_else(|| self.db.blocks.last().map(|(_, b)| b))
            .unwrap_or_default())
    }

    fn set_latest_block(&self, block: Block) -> Result<()> {
        self.cache.latest_block.write().replace(block);
        Ok(())
    }

    fn get_latest_block_header(&self) -> Result<Header> {
        self.get_latest_block().c(d!()).map(|b| b.header)
    }
}

impl TxStorage for FunStorage {
    fn insert_transactions(
        &self,
        block_height: u64,
        signed_txs: Vec<SignedTransaction>,
    ) -> Result<()> {
        let mut db = self.db.shadow();

        signed_txs
            .into_iter()
            .map(|tx| (block_height, tx))
            .for_each(|h_tx| {
                db.transactions.insert(&h_tx.1.transaction.hash, &h_tx);
                self.cache
                    .transactions
                    .insert(h_tx.1.transaction.hash, h_tx);
            });

        Ok(())
    }

    fn get_block_by_hash(&self, block_hash: &Hash) -> Result<Option<Block>> {
        if let Some(height) = self
            .cache
            .block_numbers
            .get(block_hash)
            .or_else(|| self.db.block_numbers.get(block_hash))
        {
            self.cache
                .blocks
                .get(&height)
                .or_else(|| self.db.blocks.get(&height))
                .map(Some)
                .c(d!("BUG!"))
        } else {
            Ok(None)
        }
    }

    fn get_transactions(
        &self,
        block_height: u64,
        hashes: &[Hash],
    ) -> Result<Vec<Option<SignedTransaction>>> {
        if hashes.len() > BATCH_LIMIT {
            return Err(eg!("request too large"));
        }

        Ok(hashes
            .iter()
            .map(|txh| {
                self.cache
                    .transactions
                    .get(txh)
                    .or_else(|| self.db.transactions.get(txh))
                    .and_then(|(height, tx)| {
                        alt!(height == block_height, Some(tx), None)
                    })
            })
            .collect())
    }

    fn get_transaction_by_hash(&self, hash: &Hash) -> Result<Option<SignedTransaction>> {
        Ok(self
            .cache
            .transactions
            .get(hash)
            .or_else(|| self.db.transactions.get(hash))
            .map(|(_, tx)| tx))
    }

    fn insert_receipts(&self, _block_height: u64, receipts: Vec<Receipt>) -> Result<()> {
        let mut db = self.db.shadow();

        receipts.into_iter().for_each(|r| {
            db.receipts.insert(&r.tx_hash, &r);
            self.cache.receipts.insert(r.tx_hash, r);
        });

        Ok(())
    }

    fn get_receipt_by_hash(&self, hash: &Hash) -> Result<Option<Receipt>> {
        Ok(self
            .cache
            .receipts
            .get(hash)
            .or_else(|| self.db.receipts.get(hash)))
    }

    fn get_receipts(
        &self,
        block_height: u64,
        hashes: &[Hash],
    ) -> Result<Vec<Option<Receipt>>> {
        if hashes.len() > BATCH_LIMIT {
            return Err(eg!("request too large"));
        }

        Ok(hashes
            .iter()
            .map(|txh| {
                self.cache
                    .receipts
                    .get(txh)
                    .or_else(|| self.db.receipts.get(txh))
                    .and_then(|r| alt!(r.block_number == block_height, Some(r), None))
            })
            .collect())
    }

    fn insert_code(
        &self,
        code_address: H256,
        code_hash: Hash,
        code: Vec<u8>,
    ) -> Result<()> {
        let mut db = self.db.shadow();

        db.codes_addr_to_hash
            .insert(code_address.as_bytes(), code_hash.as_bytes());
        db.codes.insert(code_hash.as_bytes(), &code);

        self.cache
            .codes_addr_to_hash
            .insert(code_address, code_hash);
        self.cache.codes.insert(code_hash, code);

        Ok(())
    }

    fn get_code_by_hash(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        Ok(self
            .cache
            .codes
            .get(hash)
            .or_else(|| self.db.codes.get(hash)))
    }

    fn get_code_by_address(&self, address: &H256) -> Result<Option<Vec<u8>>> {
        if let Some(h) = self.cache.codes_addr_to_hash.get(address).or_else(|| {
            self.db
                .codes_addr_to_hash
                .get(address)
                .map(|hash| H256::from_slice(&hash))
        }) {
            self.get_code_by_hash(&h).c(d!())
        } else {
            Ok(None)
        }
    }
}
