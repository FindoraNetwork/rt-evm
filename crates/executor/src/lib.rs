#![deny(warnings)]
#![cfg_attr(feature = "benchmark", allow(warnings))]
#![allow(clippy::uninlined_format_args, clippy::box_default)]

pub mod adapter;
mod memory;
mod precompiles;
mod utils;

use crate::precompiles::build_precompile_set;
pub use crate::{
    adapter::RTEvmExecutorAdapter,
    utils::{
        code_address, decode_revert_msg, logs_bloom, trie_root_indexed, trie_root_txs,
    },
};
use evm::{
    executor::stack::{
        MemoryStackState, PrecompileFn, StackExecutor, StackSubstateMetadata,
    },
    CreateScheme, ExitError, ExitReason, Transfer,
};
use memory::MemoryStackStateWapper;
use rt_evm_config::{CHECK_POINT_CONFIG, CURRENT_BLOCK_HEIGHT};
use rt_evm_model::{
    codec::hex_encode,
    lazy::CHAIN_ID,
    traits::{
        ApplyBackend, Backend, Executor, ExecutorAdapter as Adapter, SystemContract,
        CONSTANT_ADDR, SYSTEM_ADDR,
    },
    types::{
        data_gas_cost, Account, Config, ExecResp, Hasher, LegacyTransaction,
        SignatureComponents, SignedTransaction, TransactionAction, TxResp,
        UnverifiedTransaction, GAS_CALL_TRANSACTION, GAS_CREATE_TRANSACTION, H160, H256,
        MIN_TRANSACTION_GAS_LIMIT, U256,
    },
};
use rt_evm_storage::ethabi::{Function, Param, ParamType, StateMutability, Token};
use ruc::{eg, pnk, Result, RucResult};
use std::{collections::BTreeMap, sync::atomic::Ordering as AtoOrd};

#[derive(Debug)]
pub struct PayFee {
    /// Source address.
    pub source: H160,
    /// Target address.
    /// Transfer value.
    pub value: U256,
}

#[derive(Default)]
pub struct RTEvmExecutor;

impl Executor for RTEvmExecutor {
    // Used for query data API, this function will not modify the world state.
    fn call<B: Backend>(
        &self,
        backend: &B,
        gas_limit: u64,
        from: Option<H160>,
        to: Option<H160>,
        value: U256,
        data: Vec<u8>,
    ) -> TxResp {
        let config = Config::london();
        let metadata = StackSubstateMetadata::new(gas_limit, &config);
        let state = MemoryStackState::new(metadata, backend);
        let precompiles = build_precompile_set();
        let mut executor =
            StackExecutor::new_with_precompiles(state, &config, &precompiles);

        let base_gas = if to.is_some() {
            GAS_CALL_TRANSACTION + data_gas_cost(&data)
        } else {
            GAS_CREATE_TRANSACTION + GAS_CALL_TRANSACTION + data_gas_cost(&data)
        };

        let (exit, res) = if let Some(addr) = &to {
            executor.transact_call(
                from.unwrap_or_default(),
                *addr,
                value,
                data,
                gas_limit,
                Vec::new(),
            )
        } else {
            executor.transact_create(
                from.unwrap_or_default(),
                value,
                data,
                gas_limit,
                Vec::new(),
            )
        };

        let used_gas = executor.used_gas() + base_gas;

        TxResp {
            exit_reason: exit,
            ret: res,
            remain_gas: executor.gas(),
            gas_used: used_gas,
            fee_cost: backend
                .gas_price()
                .checked_mul(used_gas.into())
                .unwrap_or(U256::max_value()),
            logs: vec![],
            code_address: if to.is_none() {
                Some(
                    executor
                        .create_address(CreateScheme::Legacy {
                            caller: from.unwrap_or_default(),
                        })
                        .into(),
                )
            } else {
                None
            },
            removed: false,
        }
    }

    // Function execute returns exit_reason, ret_data and remain_gas.
    fn exec<B: Backend + ApplyBackend + Adapter>(
        &self,
        backend: &mut B,
        txs: &[SignedTransaction],
        system_contracts: Option<Vec<SystemContract>>,
    ) -> Result<(ExecResp, Vec<SignedTransaction>)> {
        let txs_len = txs.len();
        let mut res = Vec::with_capacity(txs_len);

        let mut tx_hashes = Vec::with_capacity(txs_len);
        let mut receipt_hashes = Vec::with_capacity(txs_len);

        let (mut gas, mut fee) = (0u64, U256::zero());
        let precompiles = build_precompile_set();
        let config = Config::london();

        let mut system_txs = vec![];
        if let Some(contracts) = system_contracts {
            for contract in contracts {
                backend.set_gas_price(contract.gas_price);
                backend.set_origin(contract.address);
                let (mut r, current_nonce) = Self::deploy_system_contract(
                    backend,
                    &config,
                    &precompiles,
                    &contract,
                )?;
                backend.commit();

                r.logs = backend.get_logs();
                gas += r.gas_used;
                fee = fee.checked_add(r.fee_cost).unwrap_or(U256::max_value());

                receipt_hashes.push(Hasher::digest(&r.ret));

                res.push(r);

                let utx = UnverifiedTransaction {
                    unsigned: rt_evm_model::types::UnsignedTransaction::Legacy(
                        LegacyTransaction {
                            nonce: current_nonce,
                            gas_price: contract.gas_price,
                            gas_limit: U256::zero(),
                            action: TransactionAction::Create,
                            value: U256::zero(),
                            data: contract.code,
                        },
                    ),
                    signature: Some(SignatureComponents {
                        r: H256::from_low_u64_be(1).as_bytes().to_vec(),
                        s: H256::from_low_u64_be(2).as_bytes().to_vec(),
                        standard_v: 37,
                    }),
                    chain_id: **CHAIN_ID.load(),
                    hash: H256::zero(),
                };

                let stx = SignedTransaction {
                    transaction: utx.calc_hash(),
                    sender: contract.address,
                    public: None,
                };
                tx_hashes.push(stx.transaction.hash);
                system_txs.push(stx);
            }
        }

        let current_height = CURRENT_BLOCK_HEIGHT.load(AtoOrd::Relaxed);
        if current_height < CHECK_POINT_CONFIG.fix_pay_fee_height {
            let mut pay_fees = vec![];
            let mut transfers = vec![];
            for tx in txs.iter() {
                backend.set_gas_price(tx.transaction.unsigned.gas_price());
                backend.set_origin(tx.sender);

                let (mut r, pay_fee, pay_value) =
                    Self::evm_exec(backend, &config, &precompiles, tx);
                println!(
                    "exec {:?} {:?} {:?} {:?}",
                    tx.transaction.hash, r, pay_fee, pay_value
                );
                pay_fees.push(pay_fee);
                transfers.extend(pay_value);
                backend.commit();

                r.logs = backend.get_logs();
                gas += r.gas_used;
                fee = fee.checked_add(r.fee_cost).unwrap_or(U256::max_value());

                tx_hashes.push(tx.transaction.hash);
                receipt_hashes.push(Hasher::digest(&r.ret));

                res.push(r);
            }
            if let Some(addr) = CONSTANT_ADDR.get() {
                for it in transfers.iter() {
                    backend.set_gas_price(U256::from(u64::MAX));
                    backend.set_origin(*SYSTEM_ADDR);
                    pnk!(Self::pay_value(
                        backend,
                        &config,
                        &precompiles,
                        *addr,
                        it.source,
                        it.target,
                        it.value,
                    ));
                    backend.commit();
                }
                for it in pay_fees.iter() {
                    backend.set_gas_price(U256::from(u64::MAX));
                    backend.set_origin(*SYSTEM_ADDR);
                    pnk!(Self::pay_fee(
                        backend,
                        &config,
                        &precompiles,
                        *addr,
                        it.source,
                        it.value
                    ));
                    backend.commit();
                }
            }
        } else {
            for tx in txs.iter() {
                backend.set_gas_price(tx.transaction.unsigned.gas_price());
                backend.set_origin(tx.sender);

                let (mut r, pay_fee, pay_values) =
                    Self::evm_exec(backend, &config, &precompiles, tx);
                println!(
                    "exec {:?} {:?} {:?} {:?}",
                    tx.transaction.hash, r, pay_fee, pay_values
                );
                if let Some(addr) = CONSTANT_ADDR.get() {
                    for it in pay_values.iter() {
                        backend.set_gas_price(U256::from(u64::MAX));
                        backend.set_origin(*SYSTEM_ADDR);
                        pnk!(Self::pay_value(
                            backend,
                            &config,
                            &precompiles,
                            *addr,
                            it.source,
                            it.target,
                            it.value,
                        ));
                    }
                    backend.set_gas_price(U256::from(u64::MAX));
                    backend.set_origin(*SYSTEM_ADDR);
                    pnk!(Self::pay_fee(
                        backend,
                        &config,
                        &precompiles,
                        *addr,
                        pay_fee.source,
                        pay_fee.value
                    ));
                }

                backend.commit();

                r.logs = backend.get_logs();
                gas += r.gas_used;
                fee = fee.checked_add(r.fee_cost).unwrap_or(U256::max_value());

                tx_hashes.push(tx.transaction.hash);
                receipt_hashes.push(Hasher::digest(&r.ret));

                res.push(r);
            }
        }

        // Get the new root, the look-like `commit` is a noop here
        let new_state_root = backend.commit();

        let transaction_root = trie_root_indexed(&tx_hashes);
        let receipt_root = trie_root_indexed(&receipt_hashes);

        Ok((
            ExecResp {
                state_root: new_state_root,
                transaction_root,
                receipt_root,
                gas_used: gas,
                fee_used: fee, // sum(<gas in tx * gas price setted by tx> ...)
                txs_resp: res,
            },
            system_txs,
        ))
    }

    fn get_account<B: Backend + Adapter>(&self, backend: &B, address: &H160) -> Account {
        backend.get_account(*address)
    }
}

impl RTEvmExecutor {
    pub fn deploy_system_contract<B: Backend + ApplyBackend + Adapter>(
        backend: &mut B,
        config: &Config,
        precompiles: &BTreeMap<H160, PrecompileFn>,
        system_contract: &SystemContract,
    ) -> Result<(TxResp, U256)> {
        let gas_limit = U256::from(u64::MAX);

        let sender = system_contract.address;

        let metadata = StackSubstateMetadata::new(gas_limit.as_u64(), config);
        let mut executor = StackExecutor::new_with_precompiles(
            MemoryStackState::new(metadata, backend),
            config,
            precompiles,
        );

        let (exit, res) = executor.transact_create(
            sender,
            U256::zero(),
            system_contract.code.clone(),
            gas_limit.as_u64(),
            vec![],
        );
        let remained_gas = executor.gas();
        let used_gas = executor.used_gas();

        let tx_gas_price = backend.gas_price();

        if exit.is_succeed() {
            let (values, logs) = executor.into_state().deconstruct();
            backend.apply(values, logs, true);
        }
        let mut account = backend.get_account(sender);
        let current_nonce = account.nonce;
        account.nonce = current_nonce + U256::one();
        backend.save_account(sender, &account);

        Ok((
            TxResp {
                exit_reason: exit,
                ret: res,
                remain_gas: remained_gas,
                gas_used: used_gas,
                fee_cost: tx_gas_price.saturating_mul(used_gas.into()),
                logs: vec![],
                code_address: Some(code_address(sender, &current_nonce)),
                removed: false,
            },
            current_nonce,
        ))
    }

    fn pay_fee<B: Backend + ApplyBackend + Adapter>(
        backend: &mut B,
        config: &Config,
        precompiles: &BTreeMap<H160, PrecompileFn>,
        contract_address: H160,
        pay_addr: H160,
        gas: U256,
    ) -> Result<()> {
        // function payFees(address payAddr, uint256 _fees)
        #[allow(deprecated)]
        let function = Function {
            name: String::from("payFees"),
            inputs: vec![
                Param {
                    name: String::from("account"),
                    kind: ParamType::Address,
                    internal_type: Some(String::from("address")),
                },
                Param {
                    name: String::from("amount"),
                    kind: ParamType::Uint(256),
                    internal_type: Some(String::from("uint256")),
                },
            ],
            outputs: vec![],
            constant: None,
            state_mutability: StateMutability::Payable,
        };
        let data = function
            .encode_input(&[Token::Address(pay_addr), Token::Uint(gas)])
            .map_err(|e| eg!(e))?;

        let gas_limit = U256::from(u64::MAX);
        let metadata = StackSubstateMetadata::new(gas_limit.as_u64(), config);
        let mut executor = StackExecutor::new_with_precompiles(
            MemoryStackState::new(metadata, backend),
            config,
            precompiles,
        );

        let (exit, res) = executor.transact_call(
            *SYSTEM_ADDR,
            contract_address,
            U256::zero(),
            data,
            gas_limit.as_u64(),
            vec![],
        );
        if exit.is_succeed() {
            let (values, logs) = executor.into_state().deconstruct();
            backend.apply(values, logs, true);
            Ok(())
        } else {
            Err(eg!(
                "payfee error:{:?} {} {}",
                exit,
                hex_encode(&res),
                String::from_utf8_lossy(&res)
            ))
        }
    }
    fn pay_value<B: Backend + ApplyBackend + Adapter>(
        backend: &mut B,
        config: &Config,
        precompiles: &BTreeMap<H160, PrecompileFn>,
        contract_address: H160,
        sending_address: H160,
        receive_address: H160,
        value: U256,
    ) -> Result<()> {
        // function payValue(address sendingAddress, address receiveAddress,uint256 value)
        #[allow(deprecated)]
        let function = Function {
            name: String::from("payValue"),
            inputs: vec![
                Param {
                    name: String::from("from"),
                    kind: ParamType::Address,
                    internal_type: Some(String::from("address")),
                },
                Param {
                    name: String::from("to"),
                    kind: ParamType::Address,
                    internal_type: Some(String::from("address")),
                },
                Param {
                    name: String::from("amount"),
                    kind: ParamType::Uint(256),
                    internal_type: Some(String::from("uint256")),
                },
            ],
            outputs: vec![],
            constant: None,
            state_mutability: StateMutability::Payable,
        };
        let data = function
            .encode_input(&[
                Token::Address(sending_address),
                Token::Address(receive_address),
                Token::Uint(value),
            ])
            .map_err(|e| eg!(e))?;

        let gas_limit = U256::from(u64::MAX);
        let metadata = StackSubstateMetadata::new(gas_limit.as_u64(), config);
        let mut executor = StackExecutor::new_with_precompiles(
            MemoryStackState::new(metadata, backend),
            config,
            precompiles,
        );

        let (exit, res) = executor.transact_call(
            *SYSTEM_ADDR,
            contract_address,
            U256::zero(),
            data,
            gas_limit.as_u64(),
            vec![],
        );
        if exit.is_succeed() {
            let (values, logs) = executor.into_state().deconstruct();
            backend.apply(values, logs, true);
            Ok(())
        } else {
            Err(eg!(
                "pay_value error:{:?} {} {}",
                exit,
                hex_encode(&res),
                String::from_utf8_lossy(&res)
            ))
        }
    }

    pub fn evm_exec<B: Backend + ApplyBackend + Adapter>(
        backend: &mut B,
        config: &Config,
        precompiles: &BTreeMap<H160, PrecompileFn>,
        tx: &SignedTransaction,
    ) -> (TxResp, PayFee, Vec<Transfer>) {
        // Deduct pre-pay gas
        let sender = tx.sender;
        let tx_gas_price = backend.gas_price();
        let gas_limit = tx.transaction.unsigned.gas_limit();
        let prepay_gas = tx_gas_price.saturating_mul(*gas_limit);

        let mut account = backend.get_account(sender);

        let current_nonce = account.nonce;

        #[cfg(not(feature = "benchmark"))]
        if tx.transaction.unsigned.nonce() != &current_nonce {
            let min_gas_limit = U256::from(MIN_TRANSACTION_GAS_LIMIT);

            let fee_cost = tx_gas_price.saturating_mul(min_gas_limit);
            account.balance = account.balance.saturating_sub(fee_cost);
            account.nonce = current_nonce + U256::one();
            backend.save_account(sender, &account);
            return (
                TxResp {
                    exit_reason: ExitReason::Error(ExitError::Other(
                        "invalid nonce".into(),
                    )),
                    gas_used: min_gas_limit.as_u64(),
                    remain_gas: u64::default(),
                    fee_cost,
                    removed: false,
                    ret: vec![],
                    logs: vec![],
                    code_address: None,
                },
                PayFee {
                    source: sender,
                    value: fee_cost,
                },
                vec![],
            );
        }

        let current_height = CURRENT_BLOCK_HEIGHT.load(AtoOrd::Relaxed);
        if current_height < CHECK_POINT_CONFIG.fix_pay_fee_height {
            account.balance = account.balance.saturating_sub(prepay_gas);
        }
        backend.save_account(sender, &account);

        let metadata = StackSubstateMetadata::new(gas_limit.as_u64(), config);

        let mut executor = StackExecutor::new_with_precompiles(
            MemoryStackStateWapper::new(metadata, backend),
            config,
            precompiles,
        );

        let access_list = tx
            .transaction
            .unsigned
            .access_list()
            .into_iter()
            .map(|x| (x.address, x.storage_keys))
            .collect::<Vec<_>>();

        let (exit, res) = match tx.transaction.unsigned.action() {
            TransactionAction::Call(addr) => executor.transact_call(
                tx.sender,
                *addr,
                *tx.transaction.unsigned.value(),
                tx.transaction.unsigned.data().to_vec(),
                gas_limit.as_u64(),
                access_list,
            ),
            TransactionAction::Create => executor.transact_create(
                tx.sender,
                *tx.transaction.unsigned.value(),
                tx.transaction.unsigned.data().to_vec(),
                gas_limit.as_u64(),
                access_list,
            ),
        };
        let transfers = executor.state().transfers.clone();

        let remained_gas = executor.gas();
        let used_gas = executor.used_gas();

        let code_addr = if tx.transaction.unsigned.action() == &TransactionAction::Create
            && exit.is_succeed()
        {
            Some(code_address(tx.sender, &current_nonce))
        } else {
            None
        };

        if exit.is_succeed() {
            let (values, logs) = executor.into_state().deconstruct();
            backend.apply(values, logs, true);
        }

        let mut account = backend.get_account(tx.sender);
        account.nonce = current_nonce + U256::one();

        let mut remain_gas = U256::zero();
        // Add remain gas
        if remained_gas != 0 {
            remain_gas = U256::from(remained_gas)
                .checked_mul(tx_gas_price)
                .unwrap_or_else(U256::max_value);
            if current_height < CHECK_POINT_CONFIG.fix_pay_fee_height {
                account.balance = account
                    .balance
                    .checked_add(remain_gas)
                    .unwrap_or_else(U256::max_value);
            }
        }

        backend.save_account(tx.sender, &account);

        (
            TxResp {
                exit_reason: exit,
                ret: res,
                remain_gas: remained_gas,
                gas_used: used_gas,
                fee_cost: tx_gas_price.saturating_mul(used_gas.into()),
                logs: vec![],
                code_address: code_addr,
                removed: false,
            },
            PayFee {
                source: sender,
                value: prepay_gas.saturating_sub(remain_gas),
            },
            transfers,
        )
    }
}
