use anyhow::Result;
use clap::{Parser, Subcommand};
use log::LevelFilter;
use simplelog::*;
use std::fs::File;
use vane_backend::VaneSwarmServer;

fn log_setup(live: bool) -> Result<(), anyhow::Error> {
    if live {
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
    } else {
        CombinedLogger::init(vec![
            TermLogger::new(
                LevelFilter::Debug,
                Config::default(),
                TerminalMode::Mixed,
                ColorChoice::Auto,
            ),
            WriteLogger::new(
                LevelFilter::Debug,
                Config::default(),
                File::create("vane.log").unwrap(),
            ),
        ])
        .unwrap();
    }
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

        /// Reading from the credentials file
        #[arg(long = "private-key-file")]
        private_key_file: Option<std::path::PathBuf>,

        /// Ed25519 private key (hex). Accepts --private-key and --private_key
        #[arg(long = "private-key", visible_alias = "private_key")]
        private_key: Option<String>,
    },
    /// Run the Vane backend server
    VaneBackend,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    match args.command {
        Commands::RelayNode {
            dns,
            port,
            live,
            private_key_file,
            private_key,
        } => {
            log_setup(live)?;

            if live && private_key_file.is_none() && private_key.is_none() {
                anyhow::bail!("No private key passed in live mode");
            }

            fn resolve_key(
                cli_key: &Option<String>,
                cli_file: &Option<std::path::PathBuf>,
            ) -> anyhow::Result<String> {
                use std::fs;
                if let Some(p) = cli_file {
                    return Ok(fs::read_to_string(p)?.trim().to_owned());
                }
                if let Some(k) = cli_key {
                    return Ok(k.trim().to_owned());
                } // dev only
                anyhow::bail!("No private key");
            }

            let private_key_opt = if live {
                Some(resolve_key(&private_key, &private_key_file)?)
            } else {
                None
            };
            vane_relay_node::MainRelayServerService::run(dns, port, live, private_key_opt).await?;
        }
        Commands::VaneBackend => {
            VaneSwarmServer::run().await?;
        }
    }
    Ok(())
}
