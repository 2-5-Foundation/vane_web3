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
        /// Relay node multi-address for P2P connection
        #[arg(short, long)]
        relay_node_multi_addr: String,
        /// Account identifier
        #[arg(short, long)]
        account: String,
        /// Network identifier
        #[arg(short, long)]
        network: String,
    },
}



#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log_setup()?;
    let args = Args::parse();

    match args.command {
        Commands::RelayNode { dns, port } => {
            relay_node::MainRelayServerService::run(dns, port).await?;
        }
        Commands::WasmNode {
            db_url,
            relay_node_multi_addr,
            account,
            network,
        } => {
            wasm_node::WasmMainServiceWorker::run(relay_node_multi_addr, account, network).await?;
        }
    }

    Ok(())
}
