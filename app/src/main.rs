use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
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
        #[arg(short, long, default_value_t = 30333)]
        port: u16,

        /// Enable live mode
        #[arg(short, long, default_value_t = false)]
        live: bool,

        /// Ed25519 private key (hex). Accepts --private-key and --private_key
        #[arg(long = "private-key", visible_alias = "private_key")]
        private_key: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log_setup()?;
    let args = Args::parse();
    match args.command {
        Commands::RelayNode {
            dns,
            port,
            live,
            private_key,
        } => {
            vane_relay_node::MainRelayServerService::run(dns, port, live, private_key).await?;
        }
    }
    Ok(())
}
