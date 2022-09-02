mod communication;
use crate::communication::p2p::server::Server;
use chrono::Local;
use clap::Parser;
use env_logger::Builder;
use log::LevelFilter;
use std::io::Write;
use std::path::PathBuf;

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

    let args = Args::parse();
    println!("Hello, world!");

    let mut s1 = Server::new("127.0.0.1:1333".parse().unwrap()).await;
    let mut s2 = Server::new("127.0.0.1:1334".parse().unwrap()).await;
    let mut s3 = Server::new("127.0.0.1:1335".parse().unwrap()).await;

    tokio::spawn(async move {
        _ = s1.run().await;
    });

    _ = tokio::spawn(async move {
        _ = match s2.connect("127.0.0.1:1333".parse().unwrap()).await {
            Ok(()) => (),
            Err(e) => {
                panic!("failed to connect {}", e);
            }
        }
    })
    .await;

    _ = tokio::spawn(async move {
        _ = match s3.connect("127.0.0.1:1333".parse().unwrap()).await {
            Ok(()) => (),
            Err(e) => {
                panic!("failed to connect {}", e);
            }
        }
    })
    .await;
}
