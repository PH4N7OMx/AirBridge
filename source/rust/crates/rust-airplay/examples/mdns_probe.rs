//! Start only the mDNS sockets, without publishing or withdrawing any services.
#![allow(clippy::print_stderr)]
struct Logger;
impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("{} {}", record.level(), record.args());
    }
    fn flush(&self) {}
}
fn main() {
    static LOGGER: Logger = Logger;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    let daemon = mdns_sd::ServiceDaemon::new().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    eprintln!(
        "status: {:?}",
        daemon
            .status()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(2))
    );
    daemon
        .shutdown()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
}
