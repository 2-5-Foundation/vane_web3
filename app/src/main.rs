#![cfg(not(target_arch = "wasm32"))]
use log::LevelFilter;
use simplelog::*;
use std::fs::File;

fn log_setup() -> Result<(), anyhow::Error> {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create("vane.log").unwrap(),
        ),
    ])
    .unwrap();
    Ok(())
}

use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Database URL to use
    #[arg(short, long)]
    pub db_url: Option<String>,
    /// port
    #[arg(short, long)]
    pub port: Option<u16>,
    #[arg(short, long)]
    pub airtable_record_id: String
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log_setup()?;
    let args = Args::parse();

    node::MainServiceWorker::run(args.db_url,args.port,args.airtable_record_id).await?;
    Ok(())
}


