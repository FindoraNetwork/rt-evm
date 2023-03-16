use hash_db::{AsHashDB, HashDB, HashDBRef, Hasher as KeyHasher, Prefix};
use ruc::*;
use serde::{Deserialize, Serialize};
use sp_trie::cache::{CacheSize, SharedTrieCache};
use vsdb::{basic::mapx_ord_rawkey::MapxOrdRawKey as Map, RawBytes, ValueEnDe};

const GB: usize = 1024 * 1024 * 1024;
const DEFAULT_SIZE: CacheSize = CacheSize::new(GB);

pub type TrieBackend = VsBackend<blake3_hasher::Blake3Hasher, Vec<u8>>;
type SharedCache = SharedTrieCache<blake3_hasher::Blake3Hasher>;

pub trait TrieVar: AsRef<[u8]> + for<'a> From<&'a [u8]> {}

impl<T> TrieVar for T where T: AsRef<[u8]> + for<'a> From<&'a [u8]> {}

// NOTE: make it `!Clone`
pub struct VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar,
{
    data: Map<Value<T>>,
    cache: Option<(SharedCache, usize)>,
    hashed_null_key: H::Out,
    original_null_key: Vec<u8>,
    null_node_data: T,
}

impl<H, T> VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar,
{
    /// Create a new `VsBackend` from the default null key/data
    pub fn new(cache_size: Option<usize>) -> Self {
        Self::from_null_node(&[0u8][..], (&[0u8][..]).into(), cache_size)
    }

    /// Create a new `VsBackend` from a given null key/data
    fn from_null_node(
        null_key: &[u8],
        null_node_data: T,
        cache_size: Option<usize>,
    ) -> Self {
        let cache = cache_size.map(|n| {
            (
                alt!(
                    0 == n,
                    SharedCache::new(DEFAULT_SIZE),
                    SharedCache::new(CacheSize::new(n))
                ),
                n,
            )
        });

        VsBackend {
            data: Map::new(),
            cache,
            hashed_null_key: H::hash(null_key),
            original_null_key: null_key.to_vec(),
            null_node_data,
        }
    }

    pub fn get_cache_hdr(&self) -> Option<&SharedCache> {
        self.cache.as_ref().map(|c| &c.0)
    }
}

impl<H, T> HashDB<H, T> for VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar + Clone + Sync + Send + PartialEq + Default,
{
    fn get(&self, key: &<H as KeyHasher>::Out, prefix: Prefix) -> Option<T> {
        if key == &self.hashed_null_key {
            return Some(self.null_node_data.clone());
        }
        let key = prefixed_key::<H>(key, prefix);
        match self.data.get(key) {
            Some(Value { v, rc }) if rc > 0 => Some(v),
            _ => None,
        }
    }

    fn contains(&self, key: &<H as KeyHasher>::Out, prefix: Prefix) -> bool {
        if key == &self.hashed_null_key {
            return true;
        }
        let key = prefixed_key::<H>(key, prefix);
        matches!(self.data.get(key), Some(Value { v: _, rc }) if rc > 0)
    }

    fn emplace(&mut self, key: <H as KeyHasher>::Out, prefix: Prefix, value: T) {
        if value == self.null_node_data {
            return;
        }

        let key = prefixed_key::<H>(&key, prefix);

        if let Some(mut old) = self.data.get_mut(&key) {
            if old.rc == 0 {
                old.v = value;
                old.rc = 1;
            } else {
                old.rc += 1;
            }
            return;
        }

        self.data.insert(key, &Value { v: value, rc: 1 });
    }

    fn insert(&mut self, prefix: Prefix, value: &[u8]) -> <H as KeyHasher>::Out {
        let v = T::from(value);
        if v == self.null_node_data {
            return self.hashed_null_key;
        }

        let key = H::hash(value);
        HashDB::emplace(self, key, prefix, v);
        key
    }

    fn remove(&mut self, key: &<H as KeyHasher>::Out, prefix: Prefix) {
        if key == &self.hashed_null_key {
            return;
        }

        let key = prefixed_key::<H>(key, prefix);
        if let Some(mut v) = self.data.get_mut(&key) {
            if v.rc > 0 {
                v.rc -= 1;
            }
        }
    }
}

impl<H, T> HashDBRef<H, T> for VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar + Clone + Sync + Send + Default + PartialEq,
{
    fn get(&self, key: &<H as KeyHasher>::Out, prefix: Prefix) -> Option<T> {
        HashDB::get(self, key, prefix)
    }
    fn contains(&self, key: &<H as KeyHasher>::Out, prefix: Prefix) -> bool {
        HashDB::contains(self, key, prefix)
    }
}

impl<H, T> AsHashDB<H, T> for VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar + Clone + Sync + Send + Default + PartialEq,
{
    fn as_hash_db(&self) -> &dyn HashDB<H, T> {
        self
    }
    fn as_hash_db_mut(&mut self) -> &mut dyn HashDB<H, T> {
        self
    }
}

// Derive a database key from hash value of the node (key) and the node prefix.
fn prefixed_key<H: KeyHasher>(key: &H::Out, prefix: Prefix) -> Vec<u8> {
    let mut prefixed_key = Vec::with_capacity(key.as_ref().len() + prefix.0.len() + 1);
    prefixed_key.extend_from_slice(prefix.0);
    if let Some(last) = prefix.1 {
        prefixed_key.push(last);
    }
    prefixed_key.extend_from_slice(key.as_ref());
    prefixed_key
}

struct Value<T> {
    v: T,
    rc: i32,
}

const RC_BYTES_NUM: usize = i32::to_be_bytes(0).len();

impl<T> ValueEnDe for Value<T>
where
    T: TrieVar,
{
    fn try_encode(&self) -> Result<RawBytes> {
        Ok(self.encode())
    }

    fn encode(&self) -> RawBytes {
        let vbytes = self.v.as_ref();
        let mut r = Vec::with_capacity(RC_BYTES_NUM + vbytes.len());
        r.extend_from_slice(&i32::to_be_bytes(self.rc));
        r.extend_from_slice(vbytes);
        r
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < RC_BYTES_NUM {
            return Err(eg!("invalid length"));
        }
        let rcbytes = <[u8; RC_BYTES_NUM]>::try_from(&bytes[..RC_BYTES_NUM]).unwrap();
        Ok(Self {
            v: T::from(&bytes[RC_BYTES_NUM..]),
            rc: i32::from_be_bytes(rcbytes),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
struct VsBackendSerde<T>
where
    T: TrieVar,
{
    data: Map<Value<T>>,
    cache_size: Option<usize>,

    original_null_key: Vec<u8>,
    null_node_data: Vec<u8>,
}

impl<H, T> From<VsBackendSerde<T>> for VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar,
{
    fn from(vbs: VsBackendSerde<T>) -> Self {
        Self {
            data: vbs.data,
            cache: vbs
                .cache_size
                .map(|n| (SharedCache::new(CacheSize::new(n)), n)),
            hashed_null_key: H::hash(&vbs.original_null_key),
            original_null_key: vbs.original_null_key,
            null_node_data: T::from(&vbs.null_node_data),
        }
    }
}

impl<H, T> From<&VsBackend<H, T>> for VsBackendSerde<T>
where
    H: KeyHasher,
    T: TrieVar,
{
    fn from(vb: &VsBackend<H, T>) -> Self {
        Self {
            data: unsafe { vb.data.shadow() },
            cache_size: vb.cache.as_ref().map(|c| c.1),
            original_null_key: vb.original_null_key.clone(),
            null_node_data: vb.null_node_data.as_ref().to_vec(),
        }
    }
}

impl<H, T> ValueEnDe for VsBackend<H, T>
where
    H: KeyHasher,
    T: TrieVar,
{
    fn try_encode(&self) -> Result<RawBytes> {
        bcs::to_bytes(&VsBackendSerde::from(self)).c(d!())
    }

    fn encode(&self) -> RawBytes {
        pnk!(self.try_encode())
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        bcs::from_bytes::<VsBackendSerde<T>>(bytes)
            .c(d!())
            .map(Self::from)
    }
}
