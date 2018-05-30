use std::collections::HashSet;
use std::hash::Hash;

use action::{Action, ActionQueue};
use message::{GossipMessage, GraftMessage, IhaveMessage, Message, PruneMessage};

#[derive(Debug)]
pub struct Node<N, M>
where
    N: Hash + Eq,
    M: Hash + Eq,
{
    node_id: N,
    eager_push_peers: HashSet<N>, // TODO: Vec?
    lazy_push_peers: HashSet<N>,
    missing: MissingMessages<N, M>,
    received_msgs: HashSet<M>,
    action_queue: ActionQueue<N, M>,
    clock: u64,
}
impl<N, M> Node<N, M>
where
    N: Hash + Eq + Clone,
    M: Hash + Eq + Clone,
{
    pub fn new(node_id: N) -> Self {
        Node {
            node_id,
            eager_push_peers: HashSet::new(),
            lazy_push_peers: HashSet::new(),
            missing: MissingMessages::new(),
            received_msgs: HashSet::new(),
            action_queue: ActionQueue::new(),
            clock: 0,
        }
    }

    pub fn handle_message(&mut self, message: Message<N, M>) {
        if !self.is_known_node(message.sender()) {
            return;
        }
        match message {
            Message::Gossip(m) => self.handle_gossip(m),
            Message::Ihave(m) => self.handle_ihave(m),
            Message::Graft(m) => self.handle_graft(m),
            Message::Prune(m) => self.handle_prune(m),
        }
    }

    pub fn handle_neighbour_up(&mut self, neighbour_node_id: N) {
        if self.node_id == neighbour_node_id {
            // TODO: metrics
            return;
        }
        self.eager_push_peers.insert(neighbour_node_id);
    }

    pub fn handle_neighbour_down(&mut self, neighbour_node_id: N) {
        self.eager_push_peers.remove(&neighbour_node_id);
        self.lazy_push_peers.remove(&neighbour_node_id);
        self.missing.handle_node_down(&neighbour_node_id);
    }

    pub fn forget_message(&mut self, message_id: &M) {
        self.received_msgs.remove(message_id);
    }

    pub fn handle_tick(&mut self) {
        self.clock += 1;
        while let Some(ihave) = self.missing.pop_expired(self.clock) {
            if !self.is_known_node(&ihave.sender) {
                continue;
            }
            self.eager_push_peers.insert(ihave.sender.clone());
            self.lazy_push_peers.remove(&ihave.sender);
            self.action_queue.send(
                ihave.sender,
                GraftMessage {
                    sender: self.node_id.clone(),
                    message_id: Some(ihave.message_id),
                    round: ihave.round,
                },
            );
        }
    }

    pub fn poll_action(&mut self) -> Option<Action<N, M>> {
        self.action_queue.pop()
    }

    fn handle_gossip(&mut self, m: GossipMessage<N, M>) {
        if self.received_msgs.contains(&m.message_id) {
            self.eager_push_peers.remove(&m.sender);
            self.lazy_push_peers.insert(m.sender.clone());
            self.action_queue
                .send(m.sender, PruneMessage::new(&self.node_id));
        } else {
            self.action_queue.deliver(m.message_id.clone());
            self.received_msgs.insert(m.message_id.clone());
            self.missing.cancel_timer(&m.message_id);

            self.eager_push(m.clone());
            self.lazy_push(m.clone());
            self.eager_push_peers.insert(m.sender.clone());
            self.lazy_push_peers.remove(&m.sender);
            self.optimize(m);
        }
    }

    fn handle_ihave(&mut self, m: IhaveMessage<N, M>) {
        if self.received_msgs.contains(&m.message_id) {
            return;
        }
        self.missing.push(m); // TODO: increase timeout if already exists
    }

    fn handle_graft(&mut self, mut m: GraftMessage<N, M>) {
        self.eager_push_peers.insert(m.sender.clone());
        self.lazy_push_peers.remove(&m.sender);
        if let Some(message_id) = m.message_id.take() {
            if self.received_msgs.contains(&message_id) {
                self.action_queue.send(
                    m.sender,
                    GossipMessage {
                        sender: self.node_id.clone(),
                        message_id,
                        round: m.round,
                    },
                );
            }
        }
    }

    fn handle_prune(&mut self, m: PruneMessage<N>) {
        self.eager_push_peers.remove(&m.sender);
        self.lazy_push_peers.insert(m.sender);
    }

    fn eager_push(&mut self, mut m: GossipMessage<N, M>) {
        let sender = m.sender;
        m.sender = self.node_id.clone();
        m.round = m.round.saturating_add(1);
        for p in self.eager_push_peers.iter().filter(|n| **n != sender) {
            self.action_queue.send(p.clone(), m.clone());
        }
    }

    fn lazy_push(&mut self, m: GossipMessage<N, M>) {
        let sender = m.sender;
        let m = IhaveMessage {
            sender: self.node_id.clone(),
            message_id: m.message_id,
            round: m.round.saturating_add(1),
        };
        for p in self.eager_push_peers.iter().filter(|n| **n != sender) {
            self.action_queue.send(p.clone(), m.clone());
        }
    }

    fn optimize(&mut self, m: GossipMessage<N, M>) {
        if let Some(ihave) = self.missing.get_by_id(&m.message_id) {
            let threshold = 3; // TODO
            if ihave.round < m.round && (m.round - ihave.round) >= threshold {
                self.action_queue.send(
                    ihave.sender.clone(),
                    GraftMessage {
                        sender: self.node_id.clone(),
                        message_id: None,
                        round: ihave.round,
                    },
                );
                self.action_queue
                    .send(m.sender, PruneMessage::new(&self.node_id));
            }
        }
    }

    fn is_known_node(&self, node_id: &N) -> bool {
        self.eager_push_peers.contains(node_id) || self.lazy_push_peers.contains(node_id)
    }
}

#[derive(Debug)]
struct MissingMessages<N, M>(::std::marker::PhantomData<(N, M)>);
impl<N, M> MissingMessages<N, M> {
    fn new() -> Self {
        MissingMessages(::std::marker::PhantomData)
    }

    fn push(&mut self, m: IhaveMessage<N, M>) {}

    fn pop_expired(&mut self, now: u64) -> Option<IhaveMessage<N, M>> {
        panic!()
    }

    fn cancel_timer(&mut self, _message_id: &M) {}

    fn handle_node_down(&mut self, _node_id: &N) {}

    fn is_empty(&self) -> bool {
        panic!()
    }

    fn contains(&self, _message_id: &M) -> bool {
        panic!()
    }

    fn get_by_id(&self, _message_id: &M) -> Option<&IhaveMessage<N, M>> {
        // NOTE: returns minimum round node
        panic!()
    }
}
