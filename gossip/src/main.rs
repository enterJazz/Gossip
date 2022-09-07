mod communication;
use chrono::Local;
use clap::Parser;
use env_logger::Builder;
use gossip::config;
use log::LevelFilter;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Sets a custom config file
    #[clap(short, long, value_parser, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    Builder::new()
        .format(|buf, record| {
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
        .filter(None, LevelFilter::Info)
        .init();

    // TODO: parse config from args
    let args = Args::parse();
    let config: config::Config;
    if let Some(config_path) = args.config {
        config = config::Config::load_config(config_path).unwrap();
    } else {
        panic!("no config file provided")
    }
}
