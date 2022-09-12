//! Represents the data item cache of the Gossip module
//! 
//! Contains the "Knowledge Base", which is a Ring Buffer of "Knowledge Items" with extra utilities.
//! Knowledge items contain data items as well as other metadata. These metadata consist of the item's id and the list of peers this item was sent to.
//! The item's id is a hash of the data item. This hash prevents the gossip module from filling knowledge base with duplicate items, as the knowledge buffer checks for duplicate entries before pushing a new entry.
//! Further, the list of peers of which the gossip module sent the knowledge item to enables the knowledge base to remove items which have been sent at least `degree` times to other peers. Upon sending the item to a new peer, the gossip module adds an entry to this list.
//! 
//! When the gossip module pushes a new knowledge item to the knowledge base, a free space within the knowledge base's internal ring buffer is used up. Upon reaching the config-specified `capacity`, new entries overwrite older entries, ensure that the knowledge base stores the most recently received items.
//! Before overwriting old items, the knowledge base performs an internal churn on the ring buffer. The knowledge base thereby removes all items which were sent to at least the config-supplied `degree` number of peers, thereby freeing up the capacity for new items.
//! 
//! Once a peer connects to the gossip instance, this instance may ask the knowledge base for all items which have not yet been sent to this peer.
//! Upon successfully having sent a knowledge item to a peer, the sending instance updates the sent knowledge items in the knowledge base with the peer ID this knowledge item was sent to. This information is used for the previously described churn as well as which items are returned to a newly connected peer.
use std::{
    collections::hash_map::DefaultHasher,
    fmt::{self, Display},
    hash::{Hash, Hasher},
};
use thiserror::Error;

use crate::communication::p2p::{message::Data, peer, peer::PeerIdentity};

/// a simple ringbuffer implementation which carries knowledge items; unsafe for multithread usage
#[derive(Debug)]
struct KnowledgeRingBuffer {
    /// vec which internally stores the knowledge items
    internal_storage: Vec<Option<KnowledgeItem>>,
    /// head of the ringbuf
    head: usize,
    /// tail of the ringbuf
    tail: usize,
}

impl KnowledgeRingBuffer {
    /// creates a new ring buf with a given capacity
    pub fn new(capacity: usize) -> Self {
        let internal_storage = (0..capacity).map(|_| None).collect();
        Self {
            internal_storage,
            head: 0,
            tail: 0,
        }
    }

    /// pushes a new item to the ring buf and moves the tail accordingly
    pub fn push(&mut self, ki: KnowledgeItem) {
        self.internal_storage[self.tail] = Some(ki);
        self.tail = (self.tail + 1) % self.internal_storage.capacity();
    }

    /// removes 
    pub fn pop(&mut self) -> Option<KnowledgeItem> {
        match self.internal_storage[self.head]
                .clone() {
                    Some(item) => {
                        self.internal_storage[self.head] = None;
                        self.head = (self.head + 1) % self.internal_storage.capacity();
                        Some(item)
                    },
                    None => None,
                }
    }

    pub fn len(&self) -> usize {
        let mut count = 0;
        for el in &self.internal_storage {
            if el.is_some() {
                count += 1;
            }
        }
        count
    }

    pub fn get_storage(&self) -> Vec<KnowledgeItem> {
        let mut storage_vec = vec![];
        for el in &self.internal_storage {
            if let Some(v) = el {
                storage_vec.push(v.clone())
            }
        }
        storage_vec
    }

    pub fn get_mut_storage(&mut self) -> Vec<&mut KnowledgeItem> {
        let mut storage_vec = vec![];
        for el in &mut self.internal_storage {
            if let Some(v) = el {
                storage_vec.push(v)
            }
        }
        storage_vec
    }
}

/// knowledge errors
#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("failed to push data item to ring buf: {:?}", el)]
    RingBufPushError { el: Data },
    #[error("failed to update item; item is not contained in ring buf: {:?}", el)]
    UpdateError { el: Data },
}

/// ring buffer which holds data received from internal modules and other peers
/// removes data once it is sent to `degree` peers
/// also removes oldest data if more than `cache_size` data are stored to make room for new data
pub struct KnowledgeBase {
    /// ring buf holding knowledge items
    rb: KnowledgeRingBuffer,
    /// max number of peers to which knowledge must be spread before knowledge item is removed
    degree: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KnowledgeItem {
    id: u64,
    data_item: Data,
    sent_to: Vec<peer::PeerIdentity>,
}

impl Display for KnowledgeItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Use `self.number` to refer to each positional data point.
        write!(
            f,
            "[{}] [send_to_peers:{}])",
            self.id,
            self.sent_to
                .iter()
                .map(|i| { peer::peer_into_str(*i).to_string() })
                .collect::<Vec<String>>()
                .join(",")
        )
    }
}

impl KnowledgeBase {
    pub fn new(capacity: usize, degree: usize) -> Self {
        let rb = KnowledgeRingBuffer::new(capacity);
        Self { rb, degree }
    }

    /// pushes data item to ringbuf if not already contained in ringbuf
    /// if ring buf is at full capacity, i.e. pushing an el removes an el, we first clean up the ring buf from any els with peers viewed > degree
    fn push_data_item(
        &mut self,
        data_item: Data,
        sent_to: Vec<peer::PeerIdentity>,
    ) -> Result<(), KnowledgeError> {
        // check if item already contained in ring buf
        let data_hash = KnowledgeItem::gen_data_item_id(&data_item);
        if self.contains(data_hash) {
            return Ok(());
        };

        let knowledge_item = KnowledgeItem::new(data_item, sent_to);

        if self.rb.len() == 0 {
            self.churn_ring_buf()?;
        }
        self.rb.push(knowledge_item.clone());
        Ok(())
    }

    /// checks if the knowledge base knows a given data item (is contained in the ring buf)
    pub fn is_known_item(&self, data_item: &Data) -> bool {
        let data_hash = KnowledgeItem::gen_data_item_id(data_item);
        self.contains(data_hash)
    }

    /// checks explicitly for the hash of the data item in the ring buf
    fn contains(&self, data_hash: u64) -> bool {
        for contained_item in self.rb.get_storage() {
            if contained_item.id == data_hash {
                return true;
            }
        }
        false
    }

    /// given a data item and a set of peers it was sent to:
    /// if data item exists in knowledge base, update the peers the item was sent to with the given peer list
    /// else if data item does not yet exist, push it along with peer list as a knowledge item to the knowledge base
    pub fn update_sent_item_to_peers(
        &mut self,
        data_item: Data,
        peers: Vec<peer::PeerIdentity>,
    ) -> Result<(), KnowledgeError> {
        let data_hash = KnowledgeItem::gen_data_item_id(&data_item);
        let mut item_updated = false;

        for ki in self.rb.get_mut_storage() {
            if &ki.id == &data_hash {
                // update peer sent_to
                ki.update_sent_to(peers.clone());
                item_updated = true;
            }
        }

        // no corresponding item exists in ringbuf; insert into ringbuf
        if !item_updated {
            self.push_data_item(data_item, peers)?;
        }
        Ok(())
    }

    /// returns all data items of knowledge items which were not sent to a given peer
    /// used when a new peer ID is discovered to send cached data items to it
    pub fn get_peer_unsent_items(&self, peer_id: peer::PeerIdentity) -> Vec<Data> {
        let mut peer_unsent_items = vec![];
        for item in self.rb.get_storage() {
            if !item.check_sent_to_peer(&peer_id) {
                peer_unsent_items.push(item.data_item.clone());
            }
        }
        peer_unsent_items
    }

    /// removes any items in ring buf where the peer list is larger than `degree`
    fn churn_ring_buf(&mut self) -> Result<(), KnowledgeError> {
        for _ in 0..self.rb.len() {
            if let Some(el) = self.rb.pop() {
                if !(el.sent_to_len() >= self.degree) {
                    self.rb.push(el)
                }
            }
        }
        Ok(())
    }
}

impl Display for KnowledgeBase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        _ = writeln!(f, "");
        _ = writeln!(
            f,
            "----------------------------------------------------------------------------"
        );
        _ = writeln!(
            f,
            "                             knowledge base                                 "
        );
        _ = writeln!(
            f,
            "----------------------------------------------------------------------------"
        );
        for item in &self.rb.get_storage() {
            _ = writeln!(f, "{}", item);
        }
        writeln!(
            f,
            "----------------------------------------------------------------------------"
        )
    }
}

impl KnowledgeItem {
    /// creates a new knowledge item from a data item and a peer list containing peers it was sent to
    fn new(data_item: Data, sent_to: Vec<peer::PeerIdentity>) -> Self {
        let id = KnowledgeItem::gen_data_item_id(&data_item);
        Self {
            id,
            data_item,
            sent_to,
        }
    }

    /// updates the peer list the item was sent to with new peers
    fn update_sent_to(&mut self, new_sent_to: Vec<peer::PeerIdentity>) {
        for new_el in new_sent_to {
            if !self.sent_to.contains(&new_el) {
                self.sent_to.push(new_el)
            }
        }
    }

    /// returns the number of unique peers the item was sent to
    fn sent_to_len(&self) -> usize {
        return self.sent_to.len();
    }

    /// check if the knowledge item was sent to a given peer
    fn check_sent_to_peer(&self, peer_id: &peer::PeerIdentity) -> bool {
        self.sent_to.contains(peer_id)
    }

    /// creates the unique id of a given data item
    fn gen_data_item_id(data_item: &Data) -> u64 {
        let mut hasher = DefaultHasher::new();
        data_item.data_type.hash(&mut hasher);
        data_item.payload.hash(&mut hasher);
        data_item.ttl.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communication::p2p::message::Data;
    use peer::PeerIdentity;
    use rand::random;

    const TEST_CAPACITY: usize = 5;
    const TEST_DEGREE: usize = 1;

    fn gen_random_data_item() -> Data {
        let payload: [u8; 30] = random();
        Data {
            ttl: random(),
            data_type: random(),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn test_knowledge_base() {
        env_logger::init();
        let mut kb = KnowledgeBase::new(TEST_CAPACITY, TEST_DEGREE);

        let data_1 = gen_random_data_item();
        let data_2 = gen_random_data_item();
        let data_3 = gen_random_data_item();
        let data_items = vec![data_1.clone(), data_2.clone(), data_3.clone()];

        for item in data_items.clone() {
            kb.update_sent_item_to_peers(item, vec![]).unwrap();
        }

        let pid = PeerIdentity::default();
        let unsent_items = kb.get_peer_unsent_items(pid);
        for item in unsent_items.clone() {
            assert!(data_items.contains(&item));
        }
        assert_eq!(data_items.len(), unsent_items.len());

        for item in data_items.clone() {
            kb.update_sent_item_to_peers(item, vec![pid]).unwrap();
        }
        let unsent_items = kb.get_peer_unsent_items(pid);
        assert!(unsent_items.is_empty());

        for _ in 0..(TEST_CAPACITY) {
            kb.update_sent_item_to_peers(gen_random_data_item(), vec![])
                .unwrap();
        }

        let unsent_items = kb.get_peer_unsent_items(pid);
        assert_eq!(unsent_items.len(), TEST_CAPACITY);
        for item in data_items.clone() {
            assert!(!unsent_items.contains(&item));
        }

        for _ in 0..TEST_CAPACITY {
            kb.update_sent_item_to_peers(gen_random_data_item(), vec![])
                .unwrap();
        }

        let new_unsent_items = kb.get_peer_unsent_items(pid);
        for unsent_item in unsent_items {
            assert!(!new_unsent_items.contains(&unsent_item));
        }
    }
}
