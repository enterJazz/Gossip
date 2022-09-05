use clap::Parser;
use env_logger;
use gossip::config;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Sets a custom config file
    #[clap(short, long, value_parser, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let args = Args::parse();
    let config: config::Config;
    if let Some(config_path) = args.config {
        config = config::Config::load_config(config_path).unwrap();
    } else {
        panic!("no config file provided")
    }
}
