use crate::types::{Block, FatBlock, Hash, Header, Receipt, SignedTransaction, H256};
use ruc::*;

pub trait BlockStorage: Send + Sync {
    fn set_block(&self, block: Block) -> Result<()>;

    fn get_block(&self, height: u64) -> Result<Option<Block>>;

    fn get_fatblock(&self, height: u64) -> Result<Option<FatBlock>>;

    fn get_block_header(&self, height: u64) -> Result<Option<Header>>;

    fn get_latest_block(&self) -> Result<Block>;

    fn set_latest_block(&self, block: Block) -> Result<()>;

    fn get_latest_block_header(&self) -> Result<Header>;
}

pub trait TxStorage {
    fn insert_txs(
        &self,
        block_height: u64,
        signed_txs: Vec<SignedTransaction>,
    ) -> Result<()>;

    fn get_block_by_hash(&self, block_hash: &Hash) -> Result<Option<Block>>;

    fn get_txs(
        &self,
        block_height: u64,
        hashes: &[Hash],
    ) -> Result<Vec<Option<SignedTransaction>>>;

    fn get_tx_by_hash(&self, hash: &Hash) -> Result<Option<SignedTransaction>>;

    fn insert_receipts(&self, block_height: u64, receipts: Vec<Receipt>) -> Result<()>;

    fn insert_code(
        &self,
        code_address: H256,
        code_hash: Hash,
        code: Vec<u8>,
    ) -> Result<()>;

    fn get_code_by_hash(&self, hash: &Hash) -> Result<Option<Vec<u8>>>;

    fn get_code_by_address(&self, address: &H256) -> Result<Option<Vec<u8>>>;

    fn get_receipt_by_hash(&self, hash: &Hash) -> Result<Option<Receipt>>;

    fn get_receipts(
        &self,
        block_height: u64,
        hashes: &[Hash],
    ) -> Result<Vec<Option<Receipt>>>;
}

pub trait Storage: BlockStorage + TxStorage {}

impl<T: BlockStorage + TxStorage> Storage for T {}
