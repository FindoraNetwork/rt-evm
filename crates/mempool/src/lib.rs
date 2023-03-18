#![deny(warnings)]
#![cfg_attr(feature = "benchmark", allow(warnings))]

use parking_lot::{Mutex, RwLock};
use rt_evm_model::types::{Hash, SignedTransaction as SignedTx, H160};
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

#[derive(Clone, Debug)]
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

    // set once, and then immutable forever
    capacity: u64,
    tx_lifetime_in_secs: u64,
}

impl TinyMempool {
    // At most 10 minutes for a tx to be alive in mempool,
    // either to be confirmed, or to be discarded
    #[cfg(not(feature = "benchmark"))]
    pub fn new_default() -> Arc<Self> {
        Self::new(2_0000, 600)
    }

    #[cfg(feature = "benchmark")]
    pub fn new_default() -> Arc<Self> {
        Self::new(200_0000, 600)
    }

    pub fn new(capacity: u64, tx_lifetime_in_secs: u64) -> Arc<Self> {
        let address_pending_cnter = Arc::new(RwLock::new(map! {}));

        let ret = Self {
            txs: Arc::new(Mutex::new(BTreeMap::new())),
            tx_lifetime_fields: Arc::new(Mutex::new(BTreeMap::new())),
            broadcast_queue: Arc::new(Mutex::new(vec![])),
            address_pending_cnter,
            stop_cleaner: Arc::new(AtomicBool::new(false)),
            capacity,
            tx_lifetime_in_secs,
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
    pub fn tx_insert(&self, tx: SignedTx) -> Result<()> {
        if self.tx_pending_cnt(None) > self.capacity {
            return Err(eg!("mempool is full"));
        }

        self.broadcast_queue.lock().push(tx.clone());

        let idx = TX_INDEXER.fetch_sub(1, AtoOrd::Relaxed);

        self.address_pending_cnter
            .write()
            .entry(tx.sender)
            .or_insert(map! {})
            .insert(tx.transaction.hash, idx);

        self.tx_lifetime_fields
            .lock()
            .insert(ts!() % self.tx_lifetime_in_secs, idx);

        self.txs.lock().insert(idx, tx);

        Ok(())
    }

    // add some new transactions to mempool
    pub fn tx_insert_batch(&self, txs: Vec<SignedTx>) -> Result<()> {
        if self.tx_pending_cnt(None) + (txs.len() as u64) >= self.capacity {
            return Err(eg!("mempool will be full after this batch"));
        }

        for tx in txs.into_iter() {
            self.tx_insert(tx).c(d!())?;
        }

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
}
