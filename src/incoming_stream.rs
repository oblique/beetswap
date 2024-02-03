use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use asynchronous_codec::FramedRead;
use cid::CidGeneric;
use fnv::FnvHashMap;
use futures::future::{BoxFuture, Fuse, FusedFuture};
use futures::{FutureExt, StreamExt};

use crate::cid_prefix::CidPrefix;
use crate::message::Codec;
use crate::multihasher::{MultihasherError, MultihasherTable};
use crate::proto::message::mod_Message::BlockPresenceType;
use crate::proto::message::mod_Message::Wantlist;

/// Stream that reads `Message` and converts it to `IncomingMessage`.
///
/// On any error `None` is returned which instruct `SelectAll` to drop the stream.
pub(crate) struct IncomingStream<const S: usize> {
    multihasher: Arc<MultihasherTable<S>>,
    stream: FramedRead<libp2p_swarm::Stream, Codec>,
    processing: Fuse<BoxFuture<'static, Option<IncomingMessage<S>>>>,
}

#[derive(Debug, Default)]
pub(crate) struct ClientMessage<const S: usize> {
    pub(crate) block_presences: FnvHashMap<CidGeneric<S>, BlockPresenceType>,
    pub(crate) blocks: FnvHashMap<CidGeneric<S>, Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct ServerMessage {
    pub(crate) wantlist: Wantlist,
}

#[derive(Debug)]
pub(crate) struct IncomingMessage<const S: usize> {
    pub(crate) client: ClientMessage<S>,
    pub(crate) server: ServerMessage,
}

impl<const S: usize> IncomingStream<S> {
    pub(crate) fn new(stream: libp2p_swarm::Stream, multihasher: Arc<MultihasherTable<S>>) -> Self {
        IncomingStream {
            multihasher,
            stream: FramedRead::new(stream, Codec),
            processing: Fuse::terminated(),
        }
    }
}

impl<const S: usize> fmt::Debug for IncomingStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IncomingStream { .. }")
    }
}

impl<const S: usize> futures::Stream for IncomingStream<S> {
    type Item = IncomingMessage<S>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<IncomingMessage<S>>> {
        loop {
            if !self.processing.is_terminated() {
                match self.processing.poll_unpin(cx) {
                    Poll::Ready(val) => return Poll::Ready(val),
                    Poll::Pending => return Poll::Pending,
                }
            }

            let msg = match self.stream.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(msg))) => msg,
                Poll::Ready(Some(Err(_))) | Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };

            let multihasher = self.multihasher.clone();

            // Convert `Message` to `IncomingMessage`. On error this future
            // returns `None` which will cause the underlying stream to close.
            self.processing = async move {
                let mut client = ClientMessage::default();

                for block_presence in msg.blockPresences {
                    let cid = match CidGeneric::try_from(block_presence.cid) {
                        Ok(cid) => cid,
                        // TODO: log it
                        Err(_) => return None,
                    };

                    client.block_presences.insert(cid, block_presence.type_pb);
                }

                for payload in msg.payload {
                    let Some(cid_prefix) = CidPrefix::from_bytes(&payload.prefix) else {
                        // TODO: log "block.prefix not decodable"
                        return None;
                    };

                    let cid = match cid_prefix.to_cid(&multihasher, &payload.data).await {
                        Ok(cid) => cid,
                        // TODO: log it
                        Err(MultihasherError::UnknownMultihashCode) => continue,
                        // TODO: log it
                        Err(_) => return None,
                    };

                    client.blocks.insert(cid, payload.data);
                }

                let server = ServerMessage {
                    wantlist: msg.wantlist.unwrap_or_default(),
                };

                Some(IncomingMessage { client, server })
            }
            .boxed()
            .fuse();
        }
    }
}
