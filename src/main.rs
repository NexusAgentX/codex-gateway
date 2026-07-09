#[tokio::main]
async fn main() -> anyhow::Result<()> {
    codex_gateway::run().await
}
