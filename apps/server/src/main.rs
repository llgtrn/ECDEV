#[tokio::main]
async fn main() {
    if let Err(e) = ecdev_server::run().await {
        eprintln!("ECDEV: {e}");
        std::process::exit(1);
    }
}
