use bytes::{BufMut, BytesMut};
use ethereum_types::BigEndianHash;
use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};

use rt_evm_crypto::secp256k1_recover;

use crate::lazy::CHAIN_ID;
use crate::types::{
    public_to_address, AccessList, AccessListItem, Bytes, Eip1559Transaction,
    Eip2930Transaction, Hasher, LegacyTransaction, Public, SignatureComponents,
    SignedTransaction, UnsignedTransaction, UnverifiedTransaction, H256, U256,
};

fn truncate_slice<T>(s: &[T], n: usize) -> &[T] {
    match s.len() {
        l if l <= n => s,
        _ => &s[0..n],
    }
}

impl Encodable for SignatureComponents {
    fn rlp_append(&self, s: &mut RlpStream) {
        if self.is_eth_sig() {
            let r = U256::from(truncate_slice(&self.r, 32));
            let s_ = U256::from(truncate_slice(&self.s, 32));
            s.append(&self.standard_v).append(&r).append(&s_);
        } else {
            s.append(&self.standard_v).append(&self.r).append(&self.s);
        }
    }
}

impl SignatureComponents {
    fn rlp_decode(
        rlp: &Rlp,
        offset: usize,
        legacy_v: Option<u64>,
    ) -> Result<Self, DecoderError> {
        let v: u8 = if let Some(n) = legacy_v {
            SignatureComponents::extract_standard_v(n)
                .ok_or(DecoderError::Custom("invalid legacy v in signature"))?
        } else {
            rlp.val_at(offset)?
        };

        let eth_tx_flag = v <= 1;
        let (r, s) = match eth_tx_flag {
            true => {
                let tmp_r: U256 = rlp.val_at(offset + 1)?;
                let tmp_s: U256 = rlp.val_at(offset + 2)?;
                (
                    <H256 as BigEndianHash>::from_uint(&tmp_r)
                        .as_bytes()
                        .to_vec(),
                    <H256 as BigEndianHash>::from_uint(&tmp_s)
                        .as_bytes()
                        .to_vec(),
                )
            }
            false => {
                let tmp_r: Bytes = rlp.val_at(offset + 1)?;
                let tmp_s: Bytes = rlp.val_at(offset + 2)?;

                if tmp_r.len() != 32 {
                    return Err(DecoderError::Custom("invalid r in signature"));
                }
                if tmp_s.len() != 32 {
                    return Err(DecoderError::Custom("invalid s in signature"));
                }

                (tmp_r, tmp_s)
            }
        };

        Ok(SignatureComponents {
            standard_v: v,
            r,
            s,
        })
    }
}

impl LegacyTransaction {
    pub fn rlp_encode(
        &self,
        rlp: &mut RlpStream,
        chain_id: Option<u64>,
        signature: Option<&SignatureComponents>,
    ) {
        let rlp_stream_len = if signature.is_some() || chain_id.is_some() {
            9
        } else {
            6
        };

        rlp.begin_list(rlp_stream_len)
            .append(&self.nonce)
            .append(&self.gas_price)
            .append(&self.gas_limit)
            .append(&self.action)
            .append(&self.value)
            .append(&self.data);

        if let Some(sig) = signature {
            rlp.append(&sig.add_chain_replay_protection(chain_id))
                .append(&U256::from(truncate_slice(&sig.r, 32)))
                .append(&U256::from(truncate_slice(&sig.s, 32)));
        } else if let Some(id) = chain_id {
            rlp.append(&id).append(&0u8).append(&0u8);
        }
    }

    fn rlp_decode(r: &Rlp) -> Result<UnverifiedTransaction, DecoderError> {
        if r.item_count()? != 9 {
            return Err(DecoderError::RlpIncorrectListLen);
        }

        let tx = LegacyTransaction {
            nonce: r.val_at(0)?,
            gas_price: r.val_at(1)?,
            gas_limit: r.val_at(2)?,
            action: r.val_at(3)?,
            value: r.val_at(4)?,
            data: r.val_at(5)?,
        };

        let v: u64 = r.val_at(6)?;
        let id = SignatureComponents::extract_chain_id(v)
            .unwrap_or_else(|| **CHAIN_ID.load());

        Ok(UnverifiedTransaction {
            unsigned: UnsignedTransaction::Legacy(tx),
            signature: Some(SignatureComponents::rlp_decode(r, 6, Some(v))?),
            chain_id: id,
            hash: Hasher::digest(r.as_raw()),
        })
    }
}

impl Eip2930Transaction {
    fn rlp_encode(
        &self,
        rlp: &mut RlpStream,
        chain_id: Option<u64>,
        signature: Option<&SignatureComponents>,
    ) {
        let rlp_stream_len = if signature.is_some() { 11 } else { 8 };
        rlp.begin_list(rlp_stream_len)
            .append(&(if let Some(id) = chain_id { id } else { 0 }))
            .append(&self.nonce)
            .append(&self.gas_price)
            .append(&self.gas_limit)
            .append(&self.action)
            .append(&self.value)
            .append(&self.data);

        rlp.begin_list(self.access_list.len());
        for access in self.access_list.iter() {
            rlp.begin_list(2);
            rlp.append(&access.address);
            rlp.begin_list(access.storage_keys.len());
            for storage_key in access.storage_keys.iter() {
                rlp.append(storage_key);
            }
        }

        if let Some(sig) = signature {
            sig.rlp_append(rlp);
        }
    }

    fn rlp_decode(r: &Rlp) -> Result<UnverifiedTransaction, DecoderError> {
        if r.item_count()? != 11 {
            return Err(DecoderError::RlpIncorrectListLen);
        }

        let id: u64 = r.val_at(0)?;
        let tx = UnsignedTransaction::Eip2930(Eip2930Transaction {
            nonce: r.val_at(1)?,
            gas_price: r.val_at(2)?,
            gas_limit: r.val_at(3)?,
            action: r.val_at(4)?,
            value: r.val_at(5)?,
            data: r.val_at(6)?,
            access_list: {
                let accl_rlp = r.at(7)?;
                let mut access_list: AccessList = Vec::new();
                for i in 0..accl_rlp.item_count()? {
                    let accounts = accl_rlp.at(i)?;
                    if accounts.item_count()? != 2 {
                        return Err(DecoderError::Custom("Unknown access list length"));
                    }

                    access_list.push(AccessListItem {
                        address: accounts.val_at(0)?,
                        storage_keys: accounts.list_at(1)?,
                    });
                }
                access_list
            },
        });

        Ok(UnverifiedTransaction {
            hash: Hasher::digest([&[tx.as_u8()], r.as_raw()].concat()),
            unsigned: tx,
            signature: Some(SignatureComponents::rlp_decode(r, 8, None)?),
            chain_id: id,
        })
    }
}

impl Eip1559Transaction {
    fn rlp_encode(
        &self,
        rlp: &mut RlpStream,
        chain_id: Option<u64>,
        signature: Option<&SignatureComponents>,
    ) {
        let rlp_stream_len = if signature.is_some() { 12 } else { 9 };
        rlp.begin_list(rlp_stream_len)
            .append(&(if let Some(id) = chain_id { id } else { 0 }))
            .append(&self.nonce)
            .append(&self.max_priority_fee_per_gas)
            .append(&self.gas_price)
            .append(&self.gas_limit)
            .append(&self.action)
            .append(&self.value)
            .append(&self.data);

        rlp.begin_list(self.access_list.len());
        for access in self.access_list.iter() {
            rlp.begin_list(2);
            rlp.append(&access.address);
            rlp.begin_list(access.storage_keys.len());
            for storage_key in access.storage_keys.iter() {
                rlp.append(storage_key);
            }
        }

        if let Some(sig) = signature {
            sig.rlp_append(rlp);
        }
    }

    fn rlp_decode(r: &Rlp) -> Result<UnverifiedTransaction, DecoderError> {
        if r.item_count()? != 12 {
            return Err(DecoderError::RlpIncorrectListLen);
        }

        let id: u64 = r.val_at(0)?;
        let tx = UnsignedTransaction::Eip1559(Eip1559Transaction {
            nonce: r.val_at(1)?,
            max_priority_fee_per_gas: r.val_at(2)?,
            gas_price: r.val_at(3)?,
            gas_limit: r.val_at(4)?,
            action: r.val_at(5)?,
            value: r.val_at(6)?,
            data: r.val_at(7)?,
            access_list: {
                let accl_rlp = r.at(8)?;
                let mut access_list: AccessList = Vec::new();
                for i in 0..accl_rlp.item_count()? {
                    let accounts = accl_rlp.at(i)?;
                    if accounts.item_count()? != 2 {
                        return Err(DecoderError::Custom("Unknown access list length"));
                    }

                    access_list.push(AccessListItem {
                        address: accounts.val_at(0)?,
                        storage_keys: accounts.list_at(1)?,
                    });
                }
                access_list
            },
        });

        Ok(UnverifiedTransaction {
            hash: Hasher::digest([&[tx.as_u8()], r.as_raw()].concat()),
            unsigned: tx,
            signature: Some(SignatureComponents::rlp_decode(r, 9, None)?),
            chain_id: id,
        })
    }
}

impl Encodable for UnverifiedTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        let chain_id = Some(self.chain_id);

        match &self.unsigned {
            UnsignedTransaction::Legacy(tx) => {
                tx.rlp_encode(s, chain_id, self.signature.as_ref())
            }
            UnsignedTransaction::Eip2930(tx) => {
                tx.rlp_encode(s, chain_id, self.signature.as_ref())
            }
            UnsignedTransaction::Eip1559(tx) => {
                tx.rlp_encode(s, chain_id, self.signature.as_ref())
            }
        };
    }

    fn rlp_bytes(&self) -> BytesMut {
        let mut ret = BytesMut::new();
        let mut s = RlpStream::new();
        self.rlp_append(&mut s);

        if !self.unsigned.is_legacy() {
            ret.put_u8(self.unsigned.as_u8());
        }

        ret.put(s.out());
        ret
    }
}

impl Decodable for UnverifiedTransaction {
    fn decode(r: &Rlp) -> Result<Self, DecoderError> {
        let raw = r.as_raw();
        let header = raw[0];

        if (header & 0x80) != 0x00 {
            return LegacyTransaction::rlp_decode(r);
        }

        match header {
            0x01 => Eip2930Transaction::rlp_decode(&Rlp::new(&raw[1..])),
            0x02 => Eip1559Transaction::rlp_decode(&Rlp::new(&raw[1..])),
            _ => Err(DecoderError::Custom("Invalid transaction header")),
        }
    }
}

impl Encodable for SignedTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.append(&self.transaction.rlp_bytes());
    }
}

impl Decodable for SignedTransaction {
    fn decode(r: &Rlp) -> Result<Self, DecoderError> {
        let utx = UnverifiedTransaction::decode(&Rlp::new(r.data()?))?;
        let public = Public::from_slice(
            &secp256k1_recover(
                utx.signature_hash(true).as_bytes(),
                utx.signature
                    .as_ref()
                    .ok_or(DecoderError::Custom("missing signature"))?
                    .as_bytes()
                    .as_ref(),
            )
            .map_err(|_| DecoderError::Custom("recover signature"))?
            .serialize_uncompressed()[1..65],
        );

        Ok(SignedTransaction {
            transaction: utx,
            sender: public_to_address(&public),
            public: Some(public),
        })
    }
}
