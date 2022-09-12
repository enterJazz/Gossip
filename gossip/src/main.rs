mod communication;
use chrono::Local;
use clap::Parser;
use env_logger::Builder;
use gossip::broadcaster::Broadcaster;
use gossip::config;
use log::{info, LevelFilter};
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// If enabled, logs to stdout
    #[clap(short, long)]
    log_to_stdout: bool,
    /// Sets a custom config file
    #[clap(short, long, value_parser, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {

    let args = Args::parse();

    let mut builder = Builder::new();
    builder.format(|buf, record| {
            writeln!(
                buf,
                "[{} {}] {}:{} - {}",
                Local::now().format("%H:%M:%S"),
                match record.level() {
                    log::Level::Error => "ERR",
                    log::Level::Warn => "WRN",
                    log::Level::Info => "INF",
                    log::Level::Debug => "DBG",
                    log::Level::Trace => "TRC",
                },
                record.file().unwrap(),
                record.line().unwrap(),
                record.args(),
            )
        })
        .filter(None, LevelFilter::Debug);
    
    if args.log_to_stdout {
        builder.target(env_logger::Target::Stdout);
    }
    
    builder.init();

    // TODO: parse config from args
    let config: config::Config;
    if let Some(config_path) = args.config {
        config = config::Config::load_config(config_path).unwrap();
    } else {
        panic!("no config file provided")
    }

    let b = Broadcaster::new(config).await;

    match b.run().await {
        Ok(_) => info!("stopping gossip"),
        Err(err) => panic!("gossip failed {}", err),
    }
}
