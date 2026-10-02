#[tokio::main]
async fn main() {
    if let Err(e) = ecdev_server::serve().await {
        eprintln!("ECDEV: {e}");
        std::process::exit(1);
    }
}
