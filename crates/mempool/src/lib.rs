#![deny(warnings)]
#![cfg_attr(feature = "benchmark", allow(warnings))]

use parking_lot::{Mutex, RwLock};
use rt_evm_model::{
    codec::ProtocolCodec,
    traits::{BlockStorage, TxStorage},
    types::{
        Account, BlockNumber, Hash, SignedTransaction as SignedTx, H160,
        MAX_BLOCK_GAS_LIMIT, MIN_TRANSACTION_GAS_LIMIT, NIL_HASH, U256,
        WORLD_STATE_META_KEY,
    },
};
use rt_evm_storage::{MptStore, Storage};
use ruc::*;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    mem,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering as AtoOrd},
        Arc,
    },
    thread,
};

// decrease from u64::MAX
static TX_INDEXER: AtomicU64 = AtomicU64::new(u64::MAX);

pub use TinyMempool as Mempool;

#[derive(Clone)]
pub struct TinyMempool {
    // if number of tx exceed the capacity, deny new txs
    //
    // NOTE: lock order number is 1
    txs: Arc<Mutex<BTreeMap<u64, SignedTx>>>,

    // key: <timestamp of tx> % <lifetime limitation>
    // value: the index of tx in `txs`
    //
    // discard_guard = tx_lifetime_fields.split_off(ts!() % <lifetime limitation> - 2)
    //
    // min_tx_index_to_discard = discard_gurad.pop_last().1
    // txs_to_discard = txs.split_off(min_tx_index_to_discard)
    //
    // decrease pending cnter based on txs_to_discard
    //
    tx_lifetime_fields: Arc<Mutex<BTreeMap<u64, u64>>>,

    // record transactions that need to be broadcasted
    broadcast_queue: Arc<Mutex<Vec<SignedTx>>>,

    // pending transactions of each account
    //
    // NOTE: lock order number is 0
    address_pending_cnter: Arc<RwLock<HashMap<H160, HashMap<Hash, u64>>>>,

    // if `true`, the background thread will exit itself.
    stop_cleaner: Arc<AtomicBool>,

    // for tx pre-check
    trie_db: Arc<MptStore>,

    // for tx pre-check
    storage: Arc<Storage>,

    cfg: TinyMempoolCfg,
}

impl TinyMempool {
    pub fn new(
        capacity: u64,
        tx_lifetime_in_secs: u64,
        tx_gas_cap: Option<u64>,
        trie_db: Arc<MptStore>,
        storage: Arc<Storage>,
    ) -> Arc<Self> {
        let address_pending_cnter = Arc::new(RwLock::new(map! {}));

        let ret = Self {
            txs: Arc::new(Mutex::new(BTreeMap::new())),
            tx_lifetime_fields: Arc::new(Mutex::new(BTreeMap::new())),
            broadcast_queue: Arc::new(Mutex::new(vec![])),
            address_pending_cnter,
            stop_cleaner: Arc::new(AtomicBool::new(false)),
            trie_db,
            storage,
            cfg: TinyMempoolCfg {
                capacity,
                tx_lifetime_in_secs,
                tx_gas_cap: tx_gas_cap.unwrap_or(MAX_BLOCK_GAS_LIMIT).into(),
            },
        };
        let ret = Arc::new(ret);

        let hdr_ret = Arc::clone(&ret);
        thread::spawn(move || {
            loop {
                sleep_ms!(tx_lifetime_in_secs * 1000);

                if hdr_ret.stop_cleaner.load(AtoOrd::Relaxed) {
                    return;
                }

                let mut ts_guard = ts!() % tx_lifetime_in_secs;
                alt!(3 > ts_guard, continue);
                ts_guard -= 2;

                let mut to_discard =
                    if let Some(mut tlf) = hdr_ret.tx_lifetime_fields.try_lock() {
                        let mut to_keep = tlf.split_off(&ts_guard);
                        mem::swap(&mut to_keep, &mut tlf);
                        to_keep // now is 'to_discard'
                    } else {
                        continue;
                    };

                let idx_gurad = if let Some((_, idx)) = to_discard.pop_last() {
                    idx
                } else {
                    continue;
                };

                // For avoiding 'dead lock',
                // we call `collect` and then `iter` again
                let to_del = hdr_ret
                    .txs
                    .lock()
                    .split_off(&idx_gurad)
                    .into_values()
                    .collect::<Vec<_>>();
                let mut pending_cnter = hdr_ret.address_pending_cnter.write();
                to_del.iter().for_each(|tx| {
                    if let Some(i) = pending_cnter.get_mut(&tx.sender) {
                        i.remove(&tx.transaction.hash);
                    }
                });
            }
        });

        ret
    }

    // Add a new transaction to mempool
    #[cfg_attr(feature = "benchmark", allow(dead_code))]
    pub fn tx_insert(&self, tx: SignedTx, signature_checked: bool) -> Result<()> {
        if self.tx_pending_cnt(None) > self.cfg.capacity {
            return Err(eg!("Mempool is full"));
        }

        if self
            .address_pending_cnter
            .read()
            .get(&tx.sender)
            .and_then(|m| m.get(&tx.transaction.hash))
            .is_some()
        {
            return Err(eg!("Already cached in mempool"));
        }

        #[cfg(not(feature = "benchmark"))]
        self.tx_pre_check(&tx, signature_checked).c(d!())?;

        self.broadcast_queue.lock().push(tx.clone());

        let idx = TX_INDEXER.fetch_sub(1, AtoOrd::Relaxed);

        self.address_pending_cnter
            .write()
            .entry(tx.sender)
            .or_insert(map! {})
            .insert(tx.transaction.hash, idx);

        self.tx_lifetime_fields
            .lock()
            .insert(ts!() % self.cfg.tx_lifetime_in_secs, idx);

        self.txs.lock().insert(idx, tx);

        Ok(())
    }

    // transactions that !maybe! have not been confirmed
    pub fn tx_pending_cnt(&self, addr: Option<H160>) -> u64 {
        if let Some(addr) = addr {
            self.address_pending_cnter
                .read()
                .get(&addr)
                .map(|i| i.len() as u64)
                .unwrap_or_default()
        } else {
            self.txs.lock().len() as u64
        }
    }

    // broadcast transactions to other nodes ?
    pub fn tx_take_broadcast(&self) -> Vec<SignedTx> {
        mem::take(&mut *self.broadcast_queue.lock())
    }

    // package some transactions for proposing a new block ?
    pub fn tx_take_propose(&self, limit: usize) -> Vec<SignedTx> {
        let mut ret = self
            .txs
            .lock()
            .iter()
            .rev()
            .take(limit)
            .map(|(_, tx)| tx.clone())
            .collect::<Vec<_>>();

        ret.sort_unstable_by(|a, b| {
            let price_cmp = b
                .transaction
                .unsigned
                .gas_price()
                .cmp(&a.transaction.unsigned.gas_price());
            if matches!(price_cmp, Ordering::Equal) {
                a.transaction
                    .unsigned
                    .nonce()
                    .cmp(b.transaction.unsigned.nonce())
            } else {
                price_cmp
            }
        });

        ret
    }

    // Remove transactions after they have been confirmed ?
    pub fn tx_cleanup(&self, to_del: &[SignedTx]) {
        let mut pending_cnter = self.address_pending_cnter.write();
        let mut txs = self.txs.lock();
        to_del.iter().for_each(|tx| {
            if let Some(i) = pending_cnter.get_mut(&tx.sender) {
                if let Some(idx) = i.remove(&tx.transaction.hash) {
                    txs.remove(&idx);
                }
            }
        });
    }

    // Pre-check the tx before execute it.
    pub fn tx_pre_check(&self, tx: &SignedTx, signature_checked: bool) -> Result<()> {
        let utx = &tx.transaction;

        let gas_price = utx.unsigned.gas_price();

        if gas_price == U256::zero() {
            return Err(eg!("The 'gas price' is zero"));
        }

        if gas_price >= U256::from(u64::MAX) {
            return Err(eg!("The 'gas price' exceeds the limition(u64::MAX)"));
        }

        let gas_limit = *utx.unsigned.gas_limit();

        if gas_limit < MIN_TRANSACTION_GAS_LIMIT.into() {
            return Err(eg!(
                "The 'gas limit' less than {}",
                MIN_TRANSACTION_GAS_LIMIT
            ));
        }

        if gas_limit > self.cfg.tx_gas_cap {
            return Err(eg!(
                "The 'gas limit' exceeds the gas capacity({})",
                self.cfg.tx_gas_cap,
            ));
        }

        utx.check_hash().c(d!())?;

        if !signature_checked && tx != &SignedTx::try_from(utx.clone()).c(d!())? {
            return Err(eg!("Signature verify failed"));
        }

        let acc = self.get_account(tx.sender, None).c(d!())?;

        if &acc.nonce > utx.unsigned.nonce() {
            return Err(eg!("Invalid nonce"));
        }

        if acc.balance < gas_price.saturating_mul(MIN_TRANSACTION_GAS_LIMIT.into()) {
            return Err(eg!("Insufficient balance to cover possible gas"));
        }

        if self.storage.get_tx_by_hash(&utx.hash).c(d!())?.is_some() {
            return Err(eg!("Historical transaction detected"));
        }

        Ok(())
    }

    fn get_account(
        &self,
        address: H160,
        number: Option<BlockNumber>,
    ) -> Result<Account> {
        let header = if let Some(n) = number {
            self.storage.get_block_header(n).c(d!())?.c(d!())?
        } else {
            self.storage.get_latest_block_header().c(d!())?
        };

        let state = self
            .trie_db
            .trie_restore(&WORLD_STATE_META_KEY, None, header.state_root.into())
            .c(d!())?;

        match state.get(address.as_bytes()).c(d!())? {
            Some(bytes) => Account::decode(bytes).c(d!()),
            None => Ok(Account {
                nonce: U256::zero(),
                balance: U256::zero(),
                storage_root: NIL_HASH,
                code_hash: NIL_HASH,
            }),
        }
    }
}

// Set once, and then immutable forever ?
#[derive(Clone)]
struct TinyMempoolCfg {
    capacity: u64,
    tx_lifetime_in_secs: u64,
    tx_gas_cap: U256, // for tx pre-check
}
