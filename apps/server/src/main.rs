fn main() {
    if let Err(e) = ecdev_server::run() {
        eprintln!("ECDEV: {e}");
        std::process::exit(1);
    }
}
