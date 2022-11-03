use std::{
    sync::{Arc}, 
    ops::Range, 
};
use cyfs_base::*;
use crate::{
    stack::{WeakStack}, 
};
use super::super::super::{
    types::*, 
    channel::protocol::v0::PieceData
};
use super::{
    encode::*, 
    stream::*, 
    download::*
};



struct CacheImpl {
    chunk: ChunkId, 
    downloader: ChunkDownloader, 
    stream_cache: ChunkStreamCache, 
}

#[derive(Clone)]
pub struct ChunkCache(Arc<CacheImpl>);

impl ChunkCache {
    pub fn new(stack: WeakStack, chunk: ChunkId) -> Self {
        let stream_cache = ChunkStreamCache::new(&chunk);
        Self(Arc::new(CacheImpl {
            downloader: ChunkDownloader::new(stack.clone(), chunk.clone(), stream_cache.clone()), 
            chunk, 
            stream_cache, 
        }))
    }

    pub fn chunk(&self) -> &ChunkId {
        &self.0.chunk
    }

    pub fn downloader(&self) -> &ChunkDownloader {
        &self.0.downloader
    }

    pub fn stream(&self) -> &ChunkStreamCache {
        &self.0.stream_cache
    }

    pub fn create_encoder(&self, desc: &ChunkEncodeDesc) -> Box<dyn ChunkEncoder> {
        StreamEncoder::new(self.stream().clone(), desc).clone_as_encoder()
    }

    pub fn exists(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start >= self.chunk().len() {
            return Some(self.chunk().len()..self.chunk().len());
        }
        if range.end == 0 {
            return Some(0..0);
        }
        let range = usize::min(range.start, self.chunk().len())..usize::min(range.end, self.chunk().len());
        let index_start = (range.start / PieceData::max_payload()) as u32;
        let index_end = ((range.end - 1) / PieceData::max_payload()) as u32;
        for index in index_start..index_end + 1 {
            if !self.stream().exists(index).unwrap() {
                return None;
            }
        }
        return Some(range);
    }

    pub async fn wait_exists<T: futures::Future<Output=BuckyError>, A: Fn() -> T>(
        &self, 
        range: Range<usize>, 
        abort: A
    ) -> BuckyResult<Range<usize>> {
        if range.start >= self.chunk().len() {
            return Ok(self.chunk().len()..self.chunk().len());
        }
        if range.end == 0 {
            return Ok(0..0);
        }
        let range = usize::min(range.start, self.chunk().len())..usize::min(range.end, self.chunk().len());
        let index_start = (range.start / PieceData::max_payload()) as u32;
        let index_end = ((range.end - 1) / PieceData::max_payload()) as u32;
        for index in index_start..index_end + 1 {
            self.stream().wait_exists(index, abort()).await?;
        }
        Ok(range)
    }
    
    pub async fn read<T: futures::Future<Output=BuckyError>, A: Fn() -> T>(
        &self, 
        offset: usize, 
        buffer: &mut [u8], 
        abort: A
    ) -> BuckyResult<usize> {
        let (desc, mut offset) = PieceDesc::from_stream_offset(PieceData::max_payload(), offset as u32);
        let (mut index, range) = desc.unwrap_as_stream();
        let mut read = 0;
        loop {
            let this_read = self.stream().async_read(
                &PieceDesc::Range(index, range), 
                offset as usize, 
                &mut buffer[read..], 
                abort()).await?;
            if this_read == 0 {
                break;
            }
            index += 1;
            read += this_read;
            offset = 0;
        }
        Ok(read)
    }
}

