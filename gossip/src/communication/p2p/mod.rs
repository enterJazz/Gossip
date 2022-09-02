// pub mod com;
pub mod peer;
pub mod server;
pub mod message {
    include!(concat!(
        env!("OUT_DIR"),
        "/gossip.communicator.p2p.message.rs"
    ));
}

mod test;
