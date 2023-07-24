use evm::{
    backend::{Apply, Backend, Basic, Log},
    executor::stack::{MemoryStackState, StackState, StackSubstateMetadata},
    ExitError, Transfer,
};
use rt_evm_model::types::{H160, H256, U256};

#[derive(Clone, Debug)]
pub struct MemoryStackStateWapper<'backend, 'config, B> {
    state: MemoryStackState<'backend, 'config, B>,
    pub(crate) transfers: Vec<Transfer>,
}

impl<'backend, 'config, B: Backend> Backend
    for MemoryStackStateWapper<'backend, 'config, B>
{
    fn gas_price(&self) -> U256 {
        self.state.gas_price()
    }
    fn origin(&self) -> H160 {
        self.state.origin()
    }
    fn block_hash(&self, number: U256) -> H256 {
        self.state.block_hash(number)
    }
    fn block_number(&self) -> U256 {
        self.state.block_number()
    }
    fn block_coinbase(&self) -> H160 {
        self.state.block_coinbase()
    }
    fn block_timestamp(&self) -> U256 {
        self.state.block_timestamp()
    }
    fn block_difficulty(&self) -> U256 {
        self.state.block_difficulty()
    }
    fn block_gas_limit(&self) -> U256 {
        self.state.block_gas_limit()
    }
    fn block_base_fee_per_gas(&self) -> U256 {
        self.state.block_base_fee_per_gas()
    }

    fn chain_id(&self) -> U256 {
        self.state.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.state.exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.state.basic(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.state.code(address)
    }

    fn storage(&self, address: H160, key: H256) -> H256 {
        self.state.storage(address, key)
    }

    fn original_storage(&self, address: H160, key: H256) -> Option<H256> {
        self.state.original_storage(address, key)
    }
}

impl<'backend, 'config, B: Backend> StackState<'config>
    for MemoryStackStateWapper<'backend, 'config, B>
{
    fn metadata(&self) -> &StackSubstateMetadata<'config> {
        self.state.metadata()
    }

    fn metadata_mut(&mut self) -> &mut StackSubstateMetadata<'config> {
        self.state.metadata_mut()
    }

    fn enter(&mut self, gas_limit: u64, is_static: bool) {
        self.state.enter(gas_limit, is_static)
    }

    fn exit_commit(&mut self) -> Result<(), ExitError> {
        self.state.exit_commit()
    }

    fn exit_revert(&mut self) -> Result<(), ExitError> {
        self.state.exit_revert()
    }

    fn exit_discard(&mut self) -> Result<(), ExitError> {
        self.state.exit_discard()
    }

    fn is_empty(&self, address: H160) -> bool {
        self.state.is_empty(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.state.deleted(address)
    }

    fn is_cold(&self, address: H160) -> bool {
        self.state.is_cold(address)
    }

    fn is_storage_cold(&self, address: H160, key: H256) -> bool {
        self.state.is_storage_cold(address, key)
    }

    fn inc_nonce(&mut self, address: H160) {
        self.state.inc_nonce(address);
    }

    fn set_storage(&mut self, address: H160, key: H256, value: H256) {
        self.state.set_storage(address, key, value)
    }

    fn reset_storage(&mut self, address: H160) {
        self.state.reset_storage(address);
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) {
        self.state.log(address, topics, data);
    }

    fn set_deleted(&mut self, address: H160) {
        self.state.set_deleted(address)
    }

    fn set_code(&mut self, address: H160, code: Vec<u8>) {
        self.state.set_code(address, code)
    }

    fn transfer(&mut self, transfer: Transfer) -> Result<(), ExitError> {
        self.transfers.push(transfer.clone());
        self.state.transfer(transfer)
    }

    fn reset_balance(&mut self, address: H160) {
        self.state.reset_balance(address)
    }

    fn touch(&mut self, address: H160) {
        self.state.touch(address)
    }
}

impl<'backend, 'config, B: Backend> MemoryStackStateWapper<'backend, 'config, B> {
    pub fn new(metadata: StackSubstateMetadata<'config>, backend: &'backend B) -> Self {
        Self {
            state: MemoryStackState::new(metadata, backend),
            transfers: vec![],
        }
    }

    #[must_use]
    pub fn deconstruct(
        self,
    ) -> (
        impl IntoIterator<Item = Apply<impl IntoIterator<Item = (H256, H256)>>>,
        impl IntoIterator<Item = Log>,
    ) {
        self.state.deconstruct()
    }
}
