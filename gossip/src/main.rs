mod communication;
use crate::communication::p2p::message;
use crate::communication::p2p::server::Server;
use chrono::Local;
use clap::Parser;
use env_logger::Builder;
use log::LevelFilter;
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

    let args = Args::parse();
    println!("Hello, world!");

    let s1 = Arc::new(Server::new("127.0.0.1:1333".parse().unwrap()).await);
    let s2 = Arc::new(Server::new("127.0.0.1:1334".parse().unwrap()).await);
    let s3 = Arc::new(Server::new("127.0.0.1:1335".parse().unwrap()).await);

    let s1_run = s1.clone();
    tokio::spawn(async move {
        _ = s1_run.run().await;
    });
    let s2_run = s2.clone();
    tokio::spawn(async move {
        _ = s2_run.run().await;
    });
    let s3_run = s3.clone();
    tokio::spawn(async move {
        _ = s3_run.run().await;
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
        };
    })
    .await;

    s1.print_conns().await;
    let msg = message::envelope::Msg::Data(message::Data {
        payload: "hello there".as_bytes().to_vec(),
    });
    s1.broadcast(msg).await;

    loop {}
    // s2.print_conns().await;
    // s3.print_conns().await;
}
