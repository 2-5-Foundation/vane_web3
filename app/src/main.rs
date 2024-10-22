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

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log_setup()?;
    node::MainServiceWorker::run().await?;
    Ok(())
}
