mod communication;
use crate::communication::p2p::message;
use crate::communication::p2p::server::Server;
use chrono::Local;
use clap::Parser;
use env_logger::Builder;
use log::LevelFilter;
use log::{error, info};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
struct Args {
    /// Sets a custom config file
    #[clap(short, long, value_parser, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] - {}",
                Local::now().format("%Y-%m-%dT%H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .filter(None, LevelFilter::Debug)
        .init();

    // TODO: parse config from args
    let args = Args::parse();
}
