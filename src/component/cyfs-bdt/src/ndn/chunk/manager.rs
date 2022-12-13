use std::{
    collections::{BTreeMap, LinkedList}, 
    sync::{RwLock, Mutex},
};
use async_std::{
    io::Cursor
};
use async_trait::async_trait;
use cyfs_base::*;
use cyfs_util::*;
use crate::{
    stack::{WeakStack, Stack},
};
use super::{
    storage::*,  
    cache::*,
    download::*
};

#[derive(Clone)]
pub struct Config {
    pub raw_caches: RawCacheConfig
}

struct ChunkDownloaders {
    mergable: Option<WeakChunkDownloader>, 
    unmergable: LinkedList<WeakChunkDownloader>
}

impl ChunkDownloaders {
    fn create_downloader(&mut self, stack: &WeakStack, cache: ChunkCache, mergable: bool) -> ChunkDownloader {
        if mergable {
            if let Some(weak) = self.mergable.as_ref() {
                if let Some(downloader) = weak.to_strong() {
                    return downloader;
                } 
            }
            let downloader = ChunkDownloader::new(stack.clone(), cache);
            self.mergable = Some(downloader.to_weak());
            downloader
        } else {
            let downloader = ChunkDownloader::new(stack.clone(), cache);
            self.unmergable.push_back(downloader.to_weak());
            downloader
        }
    }
}

struct Downloaders {
    chunk_entries: BTreeMap<ChunkId, ChunkDownloaders>
}

impl Downloaders {
    fn create_downloader(&mut self, stack: &WeakStack, cache: ChunkCache, mergable: bool) -> ChunkDownloader {
        self.chunk_entries.entry(cache.chunk().clone()).or_insert(ChunkDownloaders {
            mergable: None, 
            unmergable: Default::default()
        }).create_downloader(stack, cache, mergable)
    }
}


pub struct ChunkManager {
    stack: WeakStack, 
    store: Box<dyn ChunkReader>, 
    raw_caches: RawCacheManager, 
    caches: Mutex<BTreeMap<ChunkId, WeakChunkCache>>, 
    downloaders: RwLock<Downloaders>
}

impl std::fmt::Display for ChunkManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChunkManager:{{local:{}}}", Stack::from(&self.stack).local_device_id())
    }
}


struct EmptyChunkWrapper(Box<dyn ChunkReader>);

impl EmptyChunkWrapper {
    fn new(non_empty: Box<dyn ChunkReader>) -> Self {
        Self(non_empty)
    }
}

#[async_trait]
impl ChunkReader for EmptyChunkWrapper {
    fn clone_as_reader(&self) -> Box<dyn ChunkReader> {
        Box::new(Self(self.0.clone_as_reader()))
    }

    async fn exists(&self, chunk: &ChunkId) -> bool {
        if chunk.len() == 0 {
            true
        } else {
            self.0.exists(chunk).await
        }
    }

    async fn get(&self, chunk: &ChunkId) -> BuckyResult<Box<dyn AsyncReadWithSeek + Unpin + Send + Sync>> {
        if chunk.len() == 0 {
            Ok(Box::new(Cursor::new(vec![0u8; 0])))
        } else {
            self.0.get(chunk).await
        }
    }
}



impl ChunkManager {
    pub(crate) fn new(
        weak_stack: WeakStack, 
        store: Box<dyn ChunkReader>
    ) -> Self {
        let stack = Stack::from(&weak_stack);
        Self { 
            stack: weak_stack, 
            store: Box::new(EmptyChunkWrapper::new(store)), 
            raw_caches: RawCacheManager::new(stack.config().ndn.chunk.raw_caches.clone()), 
            caches: Mutex::new(Default::default()), 
            downloaders: RwLock::new(Downloaders { chunk_entries: Default::default() })
        }
    }

    pub fn store(&self) -> &dyn ChunkReader {
        self.store.as_ref()
    }

    pub fn raw_caches(&self) -> &RawCacheManager {
        &self.raw_caches
    }

    pub fn create_cache(&self, chunk: &ChunkId) -> ChunkCache {
        let mut caches = self.caches.lock().unwrap();
        if let Some(weak) = caches.get(chunk) {
            if let Some(cache) = weak.to_strong().clone() {
                return cache;
            }
            caches.remove(chunk);
        } 
        let cache = ChunkCache::new(self.stack.clone(), chunk.clone());
        caches.insert(chunk.clone(), cache.to_weak());
        cache
    }

    pub fn create_downloader(&self, chunk: &ChunkId, mergable: bool) -> ChunkDownloader {
        let cache = self.create_cache(chunk);
        let mut downloaders = self.downloaders.write().unwrap();
        downloaders.create_downloader(&self.stack, cache, mergable)
    }

}