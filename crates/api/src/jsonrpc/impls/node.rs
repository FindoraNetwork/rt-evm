use crate::{
    jsonrpc::{web3_types::Web3SyncStatus, RTEvmNodeRpcServer, RpcResult},
    SYNC_STATUS,
};
use jsonrpsee::core::Error;
use rt_evm_model::lazy::CHAIN_ID;
use rt_evm_model::types::{Hash, Hasher, Hex, H160, H256, U256};

pub struct NodeRpcImpl {
    version: String,
}

impl NodeRpcImpl {
    pub fn new(version: &str) -> Self {
        NodeRpcImpl {
            version: version.to_string(),
        }
    }
}

impl RTEvmNodeRpcServer for NodeRpcImpl {
    fn chain_id(&self) -> RpcResult<U256> {
        Ok((**CHAIN_ID.load()).into())
    }

    fn net_version(&self) -> RpcResult<String> {
        Ok((**CHAIN_ID.load()).to_string())
    }

    fn client_version(&self) -> RpcResult<String> {
        Ok(self.version.clone())
    }

    fn listening(&self) -> RpcResult<bool> {
        Ok(true)
    }

    // https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
    fn syncing(&self) -> RpcResult<Web3SyncStatus> {
        let s = *SYNC_STATUS.read();
        let ret = if s.current_block == s.highest_block {
            Web3SyncStatus::False
        } else {
            Web3SyncStatus::Doing(s)
        };
        Ok(ret)
    }

    fn mining(&self) -> RpcResult<bool> {
        Ok(false)
    }

    fn coinbase(&self) -> RpcResult<H160> {
        Ok(H160::default())
    }

    fn hashrate(&self) -> RpcResult<U256> {
        Ok(U256::one())
    }

    fn submit_work(&self, _nc: U256, _hash: H256, _summary: Hex) -> RpcResult<bool> {
        Ok(true)
    }

    fn submit_hashrate(&self, _hash_rate: Hex, _client_id: Hex) -> RpcResult<bool> {
        Ok(true)
    }

    fn sha3(&self, data: Hex) -> RpcResult<Hash> {
        let decode_data =
            Hex::decode(data.as_string()).map_err(|e| Error::Custom(e.to_string()))?;
        Ok(Hasher::digest(decode_data))
    }
}
