use crossbeam_queue::ArrayQueue;
use moka::{
    notification::{ConfigurationBuilder, DeliveryMode, RemovalCause},
    sync::{Cache, CacheBuilder},
};
use parking_lot::RwLock;
use rt_evm_model::types::{Hash, SignedTransaction as SignedTx, H160};
use ruc::*;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub use TinyMempool as Mempool;

#[derive(Debug)]
pub struct TinyMempool {
    txs: Cache<Hash, SignedTx>,

    // keep the order for proposing new blocks
    propose_order: ArrayQueue<Hash>,

    // record transactions that need to be broadcasted
    broadcast_queue: ArrayQueue<Hash>,

    // pending transactions of each account
    address_pending_cnter: Arc<RwLock<HashMap<H160, u64>>>,

    // set once, and then immutable forever
    capacity: u64,
}

unsafe impl Sync for TinyMempool {}
unsafe impl Send for TinyMempool {}

impl Default for TinyMempool {
    fn default() -> Self {
        // at most 10 minutes for a tx to be alive in mempool,
        // either to be confirmed, or to be discarded
        Self::new(20_0000, 600)
    }
}

impl TinyMempool {
    pub fn new(capacity: u64, tx_lifetime_in_secs: u64) -> Self {
        let address_pending_cnter = Arc::new(RwLock::new(map! {}));

        let cnter = Arc::clone(&address_pending_cnter);
        let listener = move |_: Arc<Hash>, tx: SignedTx, _: RemovalCause| {
            let mut cnter = cnter.write();
            if let Some(cnt) = cnter.get_mut(&tx.sender) {
                if 2 > *cnt {
                    cnter.remove(&tx.sender);
                } else {
                    *cnt -= 1;
                }
            }
        };
        let listener_conf = ConfigurationBuilder::default()
            .delivery_mode(DeliveryMode::Queued)
            .build();

        Self {
            txs: CacheBuilder::new(capacity)
                .time_to_live(Duration::from_secs(tx_lifetime_in_secs))
                .eviction_listener_with_conf(listener, listener_conf)
                .build(),
            propose_order: ArrayQueue::new(capacity as usize),
            broadcast_queue: ArrayQueue::new(capacity as usize),
            address_pending_cnter,
            capacity,
        }
    }

    // add a new transaction to mempool
    pub fn tx_insert(&self, tx: SignedTx) -> Result<()> {
        if self.tx_pending_cnt(None) >= self.capacity {
            return Err(eg!("mempool is full"));
        }

        self.broadcast_queue
            .push(tx.transaction.hash)
            .map_err(|e| eg!("{}: mempool is full", e))?;

        // we don't always propose blocks,
        // or even never propose, so use `force push` here
        self.propose_order.force_push(tx.transaction.hash);

        *self
            .address_pending_cnter
            .write()
            .entry(tx.sender)
            .or_insert(0) += 1;

        self.txs.insert(tx.transaction.hash, tx);

        Ok(())
    }

    // add some new transactions to mempool
    pub fn tx_insert_batch(&self, txs: Vec<SignedTx>) -> Result<()> {
        if self.tx_pending_cnt(None) + txs.len() as u64 >= self.capacity {
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
                .copied()
                .unwrap_or_default()
        } else {
            self.txs.entry_count()
        }
    }

    // broadcast transactions to other nodes ?
    pub fn tx_take_broadcast(&self, mut cap: u64) -> Vec<SignedTx> {
        let mut ret = vec![];

        while cap > 0 {
            if let Some(h) = self.broadcast_queue.pop() {
                if let Some(tx) = self.txs.get(&h) {
                    ret.push(tx);
                    cap -= 1;
                }
            } else {
                break;
            }
        }

        ret
    }

    // package some transactions for proposing a new block ?
    pub fn tx_take_propose(&self, mut cap: u64) -> Vec<SignedTx> {
        let mut ret = vec![];

        while cap > 0 {
            if let Some(h) = self.propose_order.pop() {
                if let Some(tx) = self.txs.get(&h) {
                    ret.push(tx);
                    cap -= 1;
                }
            } else {
                break;
            }
        }

        ret.sort_unstable_by_key(|tx| *tx.transaction.unsigned.nonce());

        ret
    }

    // remove transactions after they have been confirmed ?
    pub fn tx_remove(&self, tx_hashes: &[Hash]) {
        tx_hashes.iter().for_each(|h| {
            // the registered listener will decrease the pending cnter automatically
            self.txs.invalidate(h);
        });
    }
}
