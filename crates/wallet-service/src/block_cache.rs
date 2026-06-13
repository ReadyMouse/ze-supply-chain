//! In-memory compact block cache for `zcash_client_backend::sync::run`.
//!
//! The sync driver downloads compact blocks into this cache, scans them, and
//! deletes scanned ranges, so steady-state memory use is one scan batch.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use zcash_client_backend::data_api::chain::{error, BlockCache, BlockSource};
use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_protocol::consensus::BlockHeight;

#[derive(Clone, Default)]
pub struct MemBlockCache {
    blocks: Arc<Mutex<Vec<CompactBlock>>>,
}

impl MemBlockCache {
    pub fn new() -> Self {
        Self::default()
    }
}

impl BlockSource for MemBlockCache {
    type Error = Infallible;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let mut blocks: Vec<CompactBlock> = self
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| from_height.is_none_or(|h| b.height >= u64::from(u32::from(h))))
            .cloned()
            .collect();
        blocks.sort_by_key(|b| b.height);
        if let Some(limit) = limit {
            blocks.truncate(limit);
        }
        for block in blocks {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(&self, range: Option<&ScanRange>) -> Result<Option<BlockHeight>, Self::Error> {
        let blocks = self.blocks.lock().unwrap();
        Ok(blocks
            .iter()
            .filter(|b| {
                range.is_none_or(|r| {
                    r.block_range()
                        .contains(&BlockHeight::from_u32(b.height as u32))
                })
            })
            .map(|b| b.height)
            .max()
            .map(|h| BlockHeight::from_u32(h as u32)))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        let mut out: Vec<CompactBlock> = self
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| {
                range
                    .block_range()
                    .contains(&BlockHeight::from_u32(b.height as u32))
            })
            .cloned()
            .collect();
        out.sort_by_key(|b| b.height);
        Ok(out)
    }

    async fn insert(&self, mut compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        self.blocks.lock().unwrap().append(&mut compact_blocks);
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        self.blocks.lock().unwrap().retain(|b| {
            !range
                .block_range()
                .contains(&BlockHeight::from_u32(b.height as u32))
        });
        Ok(())
    }
}
