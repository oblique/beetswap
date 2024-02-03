use std::collections::VecDeque;
use std::fmt;

use async_trait::async_trait;
use libp2p_core::multihash::Multihash;
use multihash_codetable::MultihashDigest;

use crate::utils::convert_multihash;

#[derive(Debug, thiserror::Error)]
pub enum MultihasherError {
    #[error("Uknown multihash code")]
    UnknownMultihashCode,
    #[error("Invalid multihash size")]
    InvalidMultihashSize,
    #[error("Invalid data")]
    InvalidData,
}

/// Trait for producing a custom [`Multihash`].
#[async_trait]
pub trait Multihasher<const S: usize> {
    async fn hash(
        &self,
        multihash_code: u64,
        input: &[u8],
    ) -> Result<Multihash<S>, MultihasherError>;
}

/// [`Multihasher`] that uses [`multihash_codetable::Code`]
pub struct StandardMultihasher;

#[async_trait]
impl<const S: usize> Multihasher<S> for StandardMultihasher {
    async fn hash(
        &self,
        multihash_code: u64,
        input: &[u8],
    ) -> Result<Multihash<S>, MultihasherError> {
        let hasher = multihash_codetable::Code::try_from(multihash_code)
            .map_err(|_| MultihasherError::UnknownMultihashCode)?;

        let hash = hasher.digest(input);

        convert_multihash(&hash).ok_or(MultihasherError::InvalidMultihashSize)
    }
}

pub(crate) struct MultihasherTable<const S: usize> {
    multihashers: VecDeque<Box<dyn Multihasher<S> + Send + Sync + 'static>>,
}

impl<const S: usize> fmt::Debug for MultihasherTable<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MultihasherTable { .. }")
    }
}

impl<const S: usize> MultihasherTable<S> {
    pub(crate) fn new() -> Self {
        let mut table = MultihasherTable {
            multihashers: VecDeque::new(),
        };

        table.register(StandardMultihasher);

        table
    }

    pub(crate) fn register<M>(&mut self, multihasher: M)
    where
        M: Multihasher<S> + Send + Sync + 'static,
    {
        self.multihashers.push_front(Box::new(multihasher));
    }

    pub(crate) async fn hash(
        &self,
        multihash_code: u64,
        input: &[u8],
    ) -> Result<Multihash<S>, MultihasherError> {
        for multihasher in &self.multihashers {
            match multihasher.hash(multihash_code, input).await {
                Ok(hash) => return Ok(hash),
                Err(MultihasherError::UnknownMultihashCode) => continue,
                Err(e) => return Err(e),
            }
        }

        Err(MultihasherError::UnknownMultihashCode)
    }
}
