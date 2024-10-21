#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    node::MainServiceWorker::run().await?;
    Ok(())
}
