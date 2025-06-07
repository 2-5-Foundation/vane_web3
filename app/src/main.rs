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
    pub redis_url: String,
    #[arg(short, long)]
    pub account_profile_hash: String,
    /// Account pairs in format "address:network,address:network,..."
    #[arg(short, long, value_parser = parse_account_pairs)]
    pub accounts: Vec<(String, String)>,
}

fn parse_account_pairs(s: &str) -> Result<Vec<(String, String)>, String> {
    s.split(',')
        .map(|pair| {
            let parts: Vec<&str> = pair.trim().split(':').collect();
            if parts.len() != 2 {
                return Err(format!(
                    "Invalid account pair format: {}. Expected format: address:network",
                    pair
                ));
            }
            Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log_setup()?;
    let args = Args::parse();

    node::MainServiceWorker::run(
        args.db_url,
        args.port,
        args.redis_url,
        args.account_profile_hash,
        args.accounts,
    )
    .await?;
    Ok(())
}
