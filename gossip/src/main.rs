use std::path::PathBuf;
use clap::Parser;
use env_logger;

#[derive(Parser, Debug)]
struct Args {
    /// Sets a custom config file
    #[clap(short, long, value_parser, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let args = Args::parse();
    println!("Hello, world!");
}
