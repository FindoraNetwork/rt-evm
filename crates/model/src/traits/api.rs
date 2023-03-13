use crate::{
    async_trait,
    types::{
        Account, Block, BlockNumber, Hash, Header, Proposal, Receipt, SignedTransaction,
        TxResp, H160, U256,
    },
};
use ruc::*;

#[async_trait]
pub trait APIAdapter: Send + Sync {
    async fn insert_signed_tx(&self, signed_tx: SignedTransaction) -> Result<()>;

    async fn get_block_by_number(&self, height: Option<u64>) -> Result<Option<Block>>;

    async fn get_block_by_hash(&self, hash: Hash) -> Result<Option<Block>>;

    async fn get_block_header_by_number(
        &self,
        height: Option<u64>,
    ) -> Result<Option<Header>>;

    async fn get_receipt_by_tx_hash(&self, tx_hash: Hash) -> Result<Option<Receipt>>;

    async fn get_receipts_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<Receipt>>>;

    async fn get_transaction_by_hash(
        &self,
        tx_hash: Hash,
    ) -> Result<Option<SignedTransaction>>;

    async fn get_transactions_by_hashes(
        &self,
        block_number: u64,
        tx_hashes: &[Hash],
    ) -> Result<Vec<Option<SignedTransaction>>>;

    async fn get_account(
        &self,
        address: H160,
        number: Option<BlockNumber>,
    ) -> Result<Account>;

    async fn get_pending_tx_count(&self, address: H160) -> Result<U256>;

    #[allow(clippy::too_many_arguments)]
    async fn evm_call(
        &self,
        from: Option<H160>,
        to: Option<H160>,
        gas_price: Option<U256>,
        gas_limit: Option<U256>,
        value: U256,
        data: Vec<u8>,
        state_root: Hash,
        proposal: Proposal,
    ) -> Result<TxResp>;

    async fn get_code_by_hash(&self, hash: &Hash) -> Result<Option<Vec<u8>>>;

    async fn get_storage_at(
        &self,
        address: H160,
        position: U256,
        state_root: Hash,
    ) -> Result<Vec<u8>>;
}
