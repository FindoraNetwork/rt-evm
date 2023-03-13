use rt_evm::{
    api::{
        run_jsonrpc_server, set_node_sync_status, DefaultAPIAdapter as API, SyncStatus,
    },
    blockproducer::BlockProducer,
    mempool::Mempool,
    model::{
        traits::{BlockStorage as _, TxStorage as _},
        types::H160,
    },
    EvmRuntime, TokenDistributon,
};
use ruc::*;
use std::{sync::Arc, time::Duration};

// TODO: other fields ...
#[derive(Default, Clone)]
pub struct Config {
    client_version: String,

    // http rpc server
    http_listening_address: Option<String>,

    // websocket rpc server
    ws_listening_address: Option<String>,

    // storage path for the vsdb crate
    vsdb_base_dir: Option<String>,

    genesis_token_distributions: Vec<TokenDistributon>,
}

impl Config {
    ///
    /// # NOTE
    ///
    /// If vsdb has not been set outside this moduler
    /// this function should be called before any other function of this moduler!
    fn set_base_dir(&self) -> Result<()> {
        if let Some(dir) = self.vsdb_base_dir.as_ref() {
            // MUST do this operation before all!
            vsdb::vsdb_set_base_dir(dir).c(d!())?;
        }
        Ok(())
    }

    async fn run(&self) -> Result<()> {
        let rt =
            EvmRuntime::restore_or_create(&self.genesis_token_distributions).c(d!())?;

        let trie = rt.get_trie_handler();
        let storage = rt.get_storage_handler();

        let mempool = Arc::new(Mempool::default());

        let api = Arc::new(API::new(
            Arc::clone(&mempool),
            Arc::clone(&trie),
            Arc::clone(&storage),
        ));

        let cfg = self.clone();
        tokio::spawn(async move {
            pnk!(
                run_jsonrpc_server(
                    api,
                    None,
                    &cfg.client_version,
                    cfg.http_listening_address.as_deref(),
                    cfg.ws_listening_address.as_deref(),
                )
                .await
            )
        });

        self.start_consensus_engine(rt, mempool).await.c(d!())
    }

    // a fake consensus demo
    async fn start_consensus_engine(
        &self,
        evm_rt: EvmRuntime,
        mempool: Arc<Mempool>,
    ) -> Result<()> {
        let block_interval = 3; // in seconds

        let trie = evm_rt.get_trie_handler();
        let storage = evm_rt.get_storage_handler();

        loop {
            tokio::time::sleep(Duration::from_secs(block_interval)).await;

            // let web3 API to known the node status,
            // set the real status in a real production environment
            set_node_sync_status(SyncStatus::default());

            // this operation must succeed here,
            // at least one genesis block has been inserted to storage.
            let latest_header = storage.get_latest_block_header().c(d!())?;

            let blockproducer = BlockProducer {
                proposer: H160::default(), // fake value
                prev_hash: latest_header.state_root,
                block_number: latest_header.number + 1,
                block_timestamp: ts!(),
                chain_id: 0, // fake value
                mempool: &mempool,
                trie: &trie,
                storage: &storage,
            };

            // take at most 1000 transactions to propose a new block
            let txs = mempool.tx_take_propose(1000);

            let (block, receipts) = blockproducer.new_block(&txs).c(d!())?;

            storage
                .insert_transactions(block.header.number, txs)
                .c(d!())?;
            storage
                .insert_receipts(block.header.number, receipts)
                .c(d!())?;
            storage.set_block(dbg!(block)).c(d!())?;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let list = (0u64..1000)
        .map(|n| TokenDistributon::new(H160::from_low_u64_ne(n), n.into()))
        .collect();

    // Set a real config for your production environment !
    let cfg = Config {
        genesis_token_distributions: list,
        ..Default::default()
    };

    cfg.set_base_dir().c(d!())?;

    cfg.run().await
}
