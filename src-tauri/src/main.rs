#[tokio::main]
async fn main() {
    if let Err(error) = opencovibe_server::run_server().await {
        eprintln!("OpenCovibe server failed: {error}");
        std::process::exit(1);
    }
}
