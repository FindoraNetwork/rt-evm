use super::TypesError;
use crate::codec::{hex_decode, hex_encode};
pub use ethereum_types::{
    BigEndianHash, Bloom, Public, Secret, Signature, H128, H160, H256, H512, H520, H64,
    U128, U256, U512, U64,
};
use hash_db::Hasher as _;
use keccak_hasher::KeccakHasher;
use ophelia::{PublicKey, UncompressedPublicKey};
use ophelia_secp256k1::Secp256k1PublicKey;
use ruc::*;
use serde::{de, Deserialize, Serialize};
use std::{fmt, result::Result as StdResult, str::FromStr};

pub struct Hasher;

impl Hasher {
    pub fn digest(data: impl AsRef<[u8]>) -> Hash {
        Hash::from(KeccakHasher::hash(data.as_ref()))
    }
}

pub type Bytes = Vec<u8>;

pub type Hash = H256;
pub type MerkleRoot = Hash;

const ADDRESS_LEN: usize = 20;
const HEX_PREFIX: &str = "0x";
const HEX_PREFIX_UPPER: &str = "0X";

// hash(&[])
pub const NIL_DATA: H256 = H256([
    197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0,
    182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
]);

pub const NIL_HASH: H256 = NIL_DATA;

pub const KB: usize = 1024;
pub const MB: usize = 1024 * KB;
pub const GB: usize = 1024 * MB;
pub const TB: usize = 1024 * GB;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DBBytes(pub Bytes);

impl AsRef<[u8]> for DBBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hex(String);

impl Hex {
    pub fn empty() -> Self {
        Hex(String::from(HEX_PREFIX))
    }

    pub fn is_empty(&self) -> bool {
        self.0.len() == 2
    }

    pub fn encode<T: AsRef<[u8]>>(src: T) -> Self {
        let mut s = HEX_PREFIX.to_string();
        s.push_str(&hex_encode(src));
        Hex(s)
    }

    pub fn decode(s: String) -> Result<Bytes> {
        let s = if Self::is_prefixed(s.as_str()) {
            &s[2..]
        } else {
            s.as_str()
        };

        hex_decode(s).c(d!())
    }

    pub fn from_string(s: String) -> Result<Self> {
        let s = if Self::is_prefixed(s.as_str()) {
            s
        } else {
            HEX_PREFIX.to_string() + &s
        };

        let _ = hex_decode(&s[2..])?;
        Ok(Hex(s))
    }

    pub fn as_string(&self) -> String {
        self.0.to_owned()
    }

    pub fn as_string_trim0x(&self) -> String {
        (self.0[2..]).to_owned()
    }

    pub fn as_bytes(&self) -> Bytes {
        hex_decode(&self.0[2..]).expect("impossible, already checked in from_string")
    }

    fn is_prefixed(s: &str) -> bool {
        s.starts_with(HEX_PREFIX) || s.starts_with(HEX_PREFIX_UPPER)
    }
}

impl Default for Hex {
    fn default() -> Self {
        Hex(String::from("0x0000000000000000"))
    }
}

impl Serialize for Hex {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

struct HexVisitor;

impl<'de> de::Visitor<'de> for HexVisitor {
    type Value = Hex;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Expect a hex string")
    }

    fn visit_string<E>(self, v: String) -> StdResult<Self::Value, E>
    where
        E: de::Error,
    {
        Hex::from_string(v).map_err(|e| de::Error::custom(e.to_string()))
    }

    fn visit_str<E>(self, v: &str) -> StdResult<Self::Value, E>
    where
        E: de::Error,
    {
        Hex::from_string(v.to_owned()).map_err(|e| de::Error::custom(e.to_string()))
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D>(deserializer: D) -> StdResult<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_string(HexVisitor)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(pub H160);

impl Default for Address {
    fn default() -> Self {
        Address::from_hex("0x0000000000000000000000000000000000000000")
            .expect("Address must consist of 20 bytes")
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_bytes(self.0.as_bytes())
    }
}

impl Address {
    pub fn from_pubkey_bytes<B: AsRef<[u8]>>(bytes: B) -> Result<Self> {
        let compressed_pubkey_len = <Secp256k1PublicKey as PublicKey>::LENGTH;
        let uncompressed_pubkey_len =
            <Secp256k1PublicKey as UncompressedPublicKey>::LENGTH;

        let slice = bytes.as_ref();
        if slice.len() != compressed_pubkey_len && slice.len() != uncompressed_pubkey_len
        {
            return Err(TypesError::InvalidPublicKey).c(d!());
        }

        // Drop first byte
        let hash = {
            if slice.len() == compressed_pubkey_len {
                let pubkey = Secp256k1PublicKey::try_from(slice)
                    .map_err(|_| TypesError::InvalidPublicKey)
                    .c(d!())?;
                Hasher::digest(&(pubkey.to_uncompressed_bytes())[1..])
            } else {
                Hasher::digest(&slice[1..])
            }
        };

        Ok(Self::from_hash(hash))
    }

    pub fn from_hash(hash: Hash) -> Self {
        Self(H160::from_slice(&hash.as_bytes()[12..]))
    }

    pub fn from_bytes(bytes: Bytes) -> Result<Self> {
        ensure_len(bytes.len(), ADDRESS_LEN)?;
        Ok(Self(H160::from_slice(&bytes[0..20])))
    }

    pub fn as_slice(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let s = clean_0x(s)?;
        Self::from_bytes(hex_decode(s)?)
    }

    pub fn eip55(&self) -> String {
        self.to_string()
    }
}

impl FromStr for Address {
    type Err = Box<dyn RucError>;
    fn from_str(s: &str) -> Result<Self> {
        if checksum(s) != s {
            return Err(TypesError::InvalidCheckSum).c(d!());
        }

        Address::from_hex(&s.to_lowercase())
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let eip55 = checksum(&hex_encode(self.0));
        eip55.fmt(f)?;
        Ok(())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let eip55 = checksum(&hex_encode(self.0));
        eip55.fmt(f)?;
        Ok(())
    }
}

fn ensure_len(real: usize, expect: usize) -> Result<()> {
    if real != expect {
        Err(TypesError::LengthMismatch { expect, real }).c(d!())
    } else {
        Ok(())
    }
}

fn clean_0x(s: &str) -> Result<&str> {
    if s.starts_with("0x") || s.starts_with("0X") {
        Ok(&s[2..])
    } else {
        Err(TypesError::HexPrefix).c(d!())
    }
}

pub fn checksum(address: &str) -> String {
    let address = address.trim_start_matches("0x").to_lowercase();
    let address_hash = hex_encode(Hasher::digest(address.as_bytes()));

    address
        .char_indices()
        .fold(String::from("0x"), |mut acc, (index, address_char)| {
            // this cannot fail since it's Keccak256 hashed
            let n = u16::from_str_radix(&address_hash[index..index + 1], 16).unwrap();

            if n > 7 {
                // make char uppercase if ith character is 9..f
                acc.push_str(&address_char.to_uppercase().to_string())
            } else {
                // already lowercased
                acc.push(address_char)
            }

            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eip55() {
        let addr = "0x35e70c3f5a794a77efc2ec5ba964bffcc7fd2c0a";
        let eip55 = Address::from_hex(addr).unwrap();
        assert_eq!(
            eip55.to_string(),
            "0x35E70C3F5A794A77Efc2Ec5bA964BFfcC7Fd2C0a"
        );
    }

    #[test]
    fn test_hex_decode() {
        let hex = String::from("0x");
        let res = Hex::from_string(hex.clone()).unwrap();
        assert!(res.is_empty());

        let res = Hex::decode(hex).unwrap();
        assert!(res.is_empty());

        let hex = String::from("123456");
        let _ = Hex::from_string(hex.clone()).unwrap();
        let _ = Hex::decode(hex).unwrap();

        let hex = String::from("0x123f");
        let _ = Hex::from_string(hex.clone()).unwrap();
        let _ = Hex::decode(hex).unwrap();
    }

    #[test]
    fn test_hash_empty() {
        let bytes = Hex::empty();
        let hash = Hasher::digest(bytes.as_bytes());
        // println!("{:?}", Hasher::digest([]).as_bytes());
        assert_eq!(hash, NIL_DATA);
    }
}
