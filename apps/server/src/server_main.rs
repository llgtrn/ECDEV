fn main() {
    if let Err(e) = ecdev_server::serve() {
        eprintln!("ECDEV: {e}");
        std::process::exit(1);
    }
}
