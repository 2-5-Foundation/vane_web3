use anyhow::{Result, anyhow};
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

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a native node with full transaction processing capabilities
    NativeNode {
        /// Database URL to use
        #[arg(short, long)]
        db_url: Option<String>,
        /// RPC port
        #[arg(short, long)]
        port: Option<u16>,
        /// Redis URL for caching and state management
        #[arg(short, long)]
        redis_url: String,
        /// Account profile hash for authentication
        #[arg(short, long)]
        account_profile_hash: String,
        /// Account pairs in format "address:network,address:network,..."
        #[arg(short, long)]
        accounts: String,
    },
    /// Run a relay node for P2P network routing and metrics
    RelayNode {
        /// DNS address for P2P discovery
        #[arg(short, long, default_value = "0.0.0.0")]
        dns: String,
        /// P2P port for network communication
        #[arg(short, long, default_value = "30333")]
        port: u16,
    },
    /// Run a WASM node for browser and lightweight environments
    WasmNode {
        /// Database URL path
        #[arg(short, long)]
        db_url: Option<String>,
        /// P2P port for network communication
        #[arg(short, long, default_value = "30333")]
        p2p_port: u16,
        /// DNS address for P2P discovery
        #[arg(short, long, default_value = "0.0.0.0")]
        dns: String,
    },
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

    match args.command {
        Commands::NativeNode { db_url, port, redis_url, account_profile_hash, accounts } => {
            // Parse accounts string into Vec<(String, String)>
            let accounts = parse_account_pairs(&accounts)
                .map_err(|e| anyhow!("Failed to parse accounts: {}", e))?;

            native_node::MainServiceWorker::run(
                db_url,
                port,
                redis_url,
                account_profile_hash,
                accounts,
            )
            .await?;
        }
        Commands::RelayNode { dns, port } => {
            relay_node::MainRelayServerService::run(dns, port).await?;
        }
        Commands::WasmNode { db_url, p2p_port, dns } => {
            wasm_node::WasmMainServiceWorker::run(
                db_url,
                p2p_port,
                dns,
            )
            .await?;
        }
    }
    
    Ok(())
}
