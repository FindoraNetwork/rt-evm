mod backend;

use backend::{KeccakHasher as H, TrieBackend};
use rt_evm_model::types::MerkleRoot;
use ruc::*;
use serde::{Deserialize, Serialize};
use sp_trie::{
    cache::{LocalTrieCache, TrieCache},
    trie_types::{
        TrieDB, TrieDBMutBuilderV1 as TrieDBMutBuilder, TrieDBMutV1 as TrieDBMut,
    },
    LayoutV1, Trie, TrieDBBuilder, TrieHash, TrieMut,
};
use std::mem;
use vsdb::basic::mapx_ord_rawkey::MapxOrdRawKey;

#[derive(Default, Serialize, Deserialize)]
pub struct MptStore {
    // backend key ==> backend instance
    //
    // the backend key
    // - for the world state MPT, it is `[0]`
    // - for the storage MPT, it is the bytes of a H160 address
    meta: MapxOrdRawKey<TrieBackend>,
}

impl MptStore {
    pub fn new() -> Self {
        Self {
            meta: MapxOrdRawKey::new(),
        }
    }

    pub fn trie_create<'a>(
        &self,
        backend_key: &'a [u8],
        cache_size: Option<usize>,
    ) -> Result<MptOnce<'a>> {
        let backend = MptStore::new_backend(cache_size);
        self.put_backend(backend_key, &backend).c(d!())?;

        let backend = Box::into_raw(Box::new(backend));
        unsafe {
            Ok(MptOnce {
                mpt: MptMut::new(&mut *backend),
                backend: Box::from_raw(backend),
            })
        }
    }

    pub fn trie_restore<'a>(
        &self,
        backend_key: &'a [u8],
        root: MerkleRoot,
    ) -> Result<MptOnce<'a>> {
        let backend = self.get_backend(backend_key).c(d!("backend not found"))?;

        let backend = Box::into_raw(Box::new(backend));
        unsafe {
            Ok(MptOnce {
                mpt: MptMut::from_existing(&mut *backend, root),
                backend: Box::from_raw(backend),
            })
        }
    }

    fn get_backend(&self, backend_key: &[u8]) -> Option<TrieBackend> {
        self.meta.get(backend_key)
    }

    fn put_backend(&self, backend_key: &[u8], backend: &TrieBackend) -> Result<()> {
        if self.meta.contains_key(backend_key) {
            return Err(eg!("backend key already exists"));
        }
        unsafe { self.meta.shadow() }.insert(backend_key, backend);
        Ok(())
    }

    fn new_backend(cache_size: Option<usize>) -> TrieBackend {
        TrieBackend::new(cache_size)
    }
}

///
/// # NOTE
///
/// The referenced field **MUST** be placed after the field that references it,
/// this is to ensure that the `drop`s can be executed in the correct order,
/// so that UB will not occur
pub struct MptOnce<'a> {
    mpt: MptMut<'a>,

    // self-reference
    #[allow(dead_code)]
    backend: Box<TrieBackend>,
}

impl<'a> MptOnce<'a> {
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.mpt.get(key).c(d!())
    }

    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.mpt.contains(key).c(d!())
    }

    pub fn insert(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.mpt.insert(key, value).c(d!()).map(|_| ())
    }

    pub fn remove(&mut self, key: &[u8]) -> Result<()> {
        self.mpt.remove(key).c(d!()).map(|_| ())
    }

    pub fn commit(&mut self) -> MerkleRoot {
        self.mpt.commit()
    }
}

///
/// # NOTE
///
/// The referenced field **MUST** be placed after the field that references it,
/// this is to ensure that the `drop`s can be executed in the correct order,
/// so that UB will not occur
pub struct MptMut<'a> {
    trie: TrieDBMut<'a, H>,

    // self-reference
    #[allow(dead_code)]
    meta: MptMeta<'a>,
}

impl<'a> MptMut<'a> {
    // keep private !!
    pub fn new(backend: &'a mut TrieBackend) -> Self {
        let lc = backend.get_cache_hdr().map(|hdr| hdr.local_cache());
        let meta = MptMeta::new(lc, Default::default(), true);

        let trie = TrieDBMutBuilder::new(backend, unsafe { &mut *meta.root })
            .with_optional_cache(
                meta.cache
                    .as_ref()
                    .map(|c| unsafe { &mut *c.cache } as &mut dyn sp_trie::TrieCache<_>),
            )
            .build();

        Self { trie, meta }
    }

    pub fn from_existing(backend: &'a mut TrieBackend, root: MerkleRoot) -> Self {
        let lc = backend.get_cache_hdr().map(|hdr| hdr.local_cache());
        let meta = MptMeta::new(lc, root.to_fixed_bytes(), true);

        let trie = TrieDBMutBuilder::from_existing(backend, unsafe { &mut *meta.root })
            .with_optional_cache(
                meta.cache
                    .as_ref()
                    .map(|c| unsafe { &mut *c.cache } as &mut dyn sp_trie::TrieCache<_>),
            )
            .build();

        Self { trie, meta }
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.trie.get(key).c(d!())
    }

    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.trie.contains(key).c(d!())
    }

    pub fn insert(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.trie.insert(key, value).c(d!()).map(|_| ())
    }

    pub fn remove(&mut self, key: &[u8]) -> Result<()> {
        self.trie.remove(key).c(d!()).map(|_| ())
    }

    pub fn commit(&mut self) -> MerkleRoot {
        self.trie.root().into()
    }
}

///
/// # NOTE
///
/// The referenced field **MUST** be placed after the field that references it,
/// this is to ensure that the `drop`s can be executed in the correct order,
/// so that UB will not occur
pub struct MptRo<'a> {
    trie: TrieDB<'a, 'a, H>,

    // self-reference
    #[allow(dead_code)]
    meta: MptMeta<'a>,
}

impl<'a> MptRo<'a> {
    pub fn from_existing(backend: &'a TrieBackend, root: MerkleRoot) -> Self {
        let lc = backend.get_cache_hdr().map(|hdr| hdr.local_cache());
        let meta = MptMeta::new(lc, root.to_fixed_bytes(), false);

        let trie = TrieDBBuilder::new(backend, unsafe { &*meta.root })
            .with_optional_cache(
                meta.cache
                    .as_ref()
                    .map(|c| unsafe { &mut *c.cache } as &mut dyn sp_trie::TrieCache<_>),
            )
            .build();

        Self { trie, meta }
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.trie.get(key).c(d!())
    }

    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.trie.contains(key).c(d!())
    }

    pub fn root(&mut self) -> MerkleRoot {
        self.trie.root().into()
    }
}

struct MptMeta<'a> {
    // self-reference
    #[allow(dead_code)]
    cache: Option<LayeredCache<'a>>,

    // self-reference
    #[allow(dead_code)]
    root: *mut TrieRoot,
}

impl<'a> MptMeta<'a> {
    fn new(lc: Option<LocalTrieCache<H>>, root: TrieRoot, mutable: bool) -> Self {
        Self {
            cache: lc.map(|lc| LayeredCache::new(lc, alt!(mutable, None, Some(root)))),
            root: Box::into_raw(Box::new(root)),
        }
    }
}

// The raw pointers in `LayeredCache` will be dropped here
impl Drop for MptMeta<'_> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.root));
            if let Some(c) = mem::take(&mut self.cache) {
                Box::from_raw(c.cache)
                    .merge_into(&Box::from_raw(c.local_cache), *self.root);
            }
        }
    }
}

struct LayeredCache<'a> {
    // self-reference
    #[allow(dead_code)]
    cache: *mut TrieCache<'a, H>,

    // self-reference
    #[allow(dead_code)]
    local_cache: *mut LocalTrieCache<H>,
}

impl<'a> LayeredCache<'a> {
    fn new(lc: LocalTrieCache<H>, root: Option<TrieRoot>) -> Self {
        let lc = Box::into_raw(Box::new(lc));

        let cache = if let Some(root) = root {
            Box::into_raw(Box::new(unsafe { &*lc }.as_trie_db_cache(root)))
        } else {
            Box::into_raw(Box::new(unsafe { &*lc }.as_trie_db_mut_cache()))
        };

        Self {
            cache,
            local_cache: lc,
        }
    }
}

type TrieRoot = TrieHash<LayoutV1<H>>;
