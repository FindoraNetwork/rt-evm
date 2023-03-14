use rt_evm::{
    api::{set_node_sync_status, SyncStatus},
    Address, EvmRuntime, TokenDistributon,
};
use ruc::*;
use std::time::Duration;

// TODO: other fields ...
#[derive(Default, Clone)]
pub struct Config {
    chain_id: u64,
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
        let rt = EvmRuntime::restore_or_create(
            self.chain_id,
            &self.genesis_token_distributions,
        )
        .c(d!())?;

        // will NOT block current thread
        rt.spawn_jsonrpc_server(
            self.client_version.as_str(),
            self.http_listening_address.as_deref(),
            self.ws_listening_address.as_deref(),
        )
        .await
        .c(d!())?;

        // the inner loop will block current thread !
        self.start_consensus_engine(&rt).await.c(d!())
    }

    // a fake consensus demo
    async fn start_consensus_engine(&self, evm_rt: &EvmRuntime) -> Result<()> {
        let block_interval = 3; // in seconds

        loop {
            tokio::time::sleep(Duration::from_secs(block_interval)).await;

            // let web3 API to known the node status,
            // should be set to a real value in production environments.
            //
            // SEE ALSO:
            // https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
            set_node_sync_status(SyncStatus::default());

            let producer = evm_rt.generate_blockproducer(select_proposer()).c(d!())?;

            // take at most 1000 transactions to propose a new block
            let txs = evm_rt.mempool_handler().tx_take_propose(1000);

            let header = producer.generate_block_and_persist(txs).c(d!())?;
            dbg!(header);
        }
    }
}

// fake
fn select_proposer() -> Address {
    Address::random()
}

#[tokio::main]
async fn main() -> Result<()> {
    let list = (0u64..1000)
        .map(|n| TokenDistributon::new(Address::from_low_u64_ne(n), n.into()))
        .collect();

    // Set a real config for your production environment !
    let cfg = Config {
        chain_id: 9527,
        genesis_token_distributions: list,
        http_listening_address: Some("localhost:60001".to_owned()),
        ws_listening_address: Some("localhost:60002".to_owned()),
        ..Default::default()
    };

    cfg.set_base_dir().c(d!())?;

    cfg.run().await
}
