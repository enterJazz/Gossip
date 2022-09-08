use std::{collections::hash_map::DefaultHasher, hash::{Hasher, Hash}};

use crate::communication::p2p::{peer, message::Data};

use ringbuf;
use thiserror::Error;

/// knowledge errors
#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("failed to push data item to ring buf: {:?}", el)]
    RingBufPushError {
        el: Data,
    },
    #[error("failed to update item; item is not contained in ring buf: {:?}", el)]
    UpdateError {
        el: Data,
    }
}

/// ring buffer which holds data received from internal modules and other peers
/// removes data once it is sent to `degree` peers
/// also removes oldest data if more than `cache_size` data are stored to make room for new data
pub struct KnowledgeBase {
    /// internal structure to hold knowledge items (data): producer
    rb_prod: ringbuf::Producer<KnowledgeItem>,
    /// internal structure to hold knowledge items (data): consumer
    rb_cons: ringbuf::Consumer<KnowledgeItem>,
    /// max number of peers to which knowledge must be spread before knowledge item is removed
    degree: usize,
}

#[derive(Debug)]
struct KnowledgeItem {
    id: u64,
    data_item: Data,
    sent_to: Vec<peer::PeerIdentity>,
}

impl KnowledgeBase {
    pub async fn new(capacity: usize, degree: usize) -> Self {
        let rb = ringbuf::RingBuffer::<KnowledgeItem>::new(capacity);
        let (rb_prod, rb_cons) = rb.split();
        Self {
            rb_prod,
            rb_cons,
            degree,
        }
    }
    
    /// pushes data item to ringbuf if not already contained in ringbuf
    /// if ring buf is at full capacity, i.e. pushing an el removes an el, we first clean up the ring buf from any els with peers viewed > 20
    pub async fn push_data_item(&mut self, data_item: Data, sent_to: Vec<peer::PeerIdentity>) -> Result<(), KnowledgeError> {

        // check if item already contained in ring buf
        let data_hash = KnowledgeItem::gen_data_item_id(&data_item).await;
        if self.rb_cons.find(|x| x.id == data_hash).is_some() {
            return Ok(());
        };

        let knowledge_item = KnowledgeItem::new(data_item, sent_to).await;

        if self.rb_cons.remaining() == 0 {
            self.churn_ring_buf().await?;
        }
        self.rb_prod.push(knowledge_item).map_err(|ki| KnowledgeError::RingBufPushError { el: ki.data_item })?;
        Ok(())
    }

    pub async fn update_sent_item_to_peers(&mut self, data_item: Data, peers: Vec<peer::PeerIdentity>) -> Result<(), KnowledgeError> {
        let data_hash = KnowledgeItem::gen_data_item_id(&data_item).await;
        let mut item_updated = false;
        
        for ki in self.rb_cons.iter_mut() {
            if &ki.id == &data_hash {
                // update peer sent_to
                ki.update_sent_to(peers.clone()).await;
                item_updated = true;
            }
        }

        // no corresponding item exists in ringbuf; insert into ringbuf
        if !item_updated {
            self.push_data_item(data_item, peers).await?;
        }
        Ok(())
    }

    /// returns all data items of knowledge items which were not sent to a given peer
    /// used when a new peer ID is discovered to send cached data items to it
    pub async fn get_peer_unsent_items(&self, peer_id: peer::PeerIdentity) -> Vec<Data> {
        let mut peer_unsent_items = vec![];
        for item in self.rb_cons.iter() {
            if !item.check_sent_to_peer(&peer_id).await {
                peer_unsent_items.push(item.data_item.clone());
            }
        }
        peer_unsent_items
    }

    /// removes any items in ring buf where the peer list is larger than `degree`
    async fn churn_ring_buf(&mut self) -> Result<(), KnowledgeError> {
        for _ in 0..self.rb_cons.len() {
            if let Some(el) = self.rb_cons.pop() {
                if !(el.sent_to_len().await >= self.degree) {
                    self.rb_prod.push(el).map_err(|ki| KnowledgeError::RingBufPushError { el: ki.data_item })?;
                }
            }
        }
        Ok(())
    }
}


impl KnowledgeItem {
    async fn new(data_item: Data, sent_to: Vec<peer::PeerIdentity>) -> Self {
        let id = KnowledgeItem::gen_data_item_id(&data_item).await;
        Self {
            id,
            data_item,
            sent_to,
        }
    }

    async fn update_sent_to(&mut self, new_sent_to: Vec<peer::PeerIdentity>) {
        for new_el in new_sent_to {
            if !self.sent_to.contains(&new_el) {
                self.sent_to.push(new_el)
            }
        }
    }

    async fn sent_to_len(&self) -> usize {
        return self.sent_to.len()
    }

    /// check if the knowledge item was sent to a given peer
    async fn check_sent_to_peer(&self, peer_id: &peer::PeerIdentity) -> bool {
        self.sent_to.contains(peer_id)
    }

    async fn gen_data_item_id(data_item: &Data) -> u64 {
        let mut hasher = DefaultHasher::new();
        data_item.data_type.hash(&mut hasher);
        data_item.payload.hash(&mut hasher);
        data_item.ttl.hash(&mut hasher);
        hasher.finish()
    }
}