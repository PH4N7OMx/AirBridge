// SPDX-License-Identifier: GPL-3.0-or-later
//! Registering and withdrawing the records through `mdns-sd`.

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};
use std::{
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use super::advertisement::{AIRPLAY_SERVICE, Advertisement, RAOP_SERVICE, hardware_address};

/// Why the receiver could not be advertised.
#[derive(Debug)]
pub enum DiscoveryError {
    /// The responder could not be started.
    Daemon(String),
    /// A record could not be built from its fields.
    Service(String),
    /// A record could not be registered.
    Register(String),
    /// A record could not be withdrawn.
    Unregister(String),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Daemon(message) => write!(formatter, "the responder did not start: {message}"),
            Self::Service(message) => {
                write!(formatter, "the record is not advertisable: {message}")
            }
            Self::Register(message) => {
                write!(formatter, "the record was not registered: {message}")
            }
            Self::Unregister(message) => {
                write!(formatter, "the record was not withdrawn: {message}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// A receiver that is announced over multicast DNS.
///
/// Dropping the value withdraws both records, so a caller keeps it alive for as
/// long as the receiver runs.
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullnames: Vec<String>,
    maintenance: Option<(mpsc::Sender<()>, JoinHandle<()>)>,
}

impl Advertiser {
    /// Announces `advertisement` on every interface the responder finds.
    ///
    /// Dropping the returned value withdraws both records.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError`] when the responder cannot start or a record
    /// cannot be built or registered, for example when a field is not a valid
    /// DNS label.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rust_airplay::discovery::{Advertisement, Advertiser, Protection};
    /// use rust_airplay::server::DeviceIdentity;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let identity = DeviceIdentity::default();
    /// let advertisement = Advertisement::new(&identity, 7000, Protection::None);
    /// let advertiser = Advertiser::register(&advertisement)?;
    /// // The receiver runs here; dropping `advertiser` withdraws it.
    /// drop(advertiser);
    /// # Ok(())
    /// # }
    /// ```
    pub fn register(advertisement: &Advertisement) -> Result<Self, DiscoveryError> {
        let daemon =
            ServiceDaemon::new().map_err(|error| DiscoveryError::Daemon(error.to_string()))?;
        let host = format!(
            "{}.local.",
            hardware_address(&advertisement.raop_name).to_lowercase()
        );
        let mut fullnames = Vec::with_capacity(2);
        let events = daemon
            .monitor()
            .map_err(|error| DiscoveryError::Daemon(error.to_string()))?;
        let mut registered = Vec::with_capacity(2);
        let records = [
            (
                RAOP_SERVICE,
                &advertisement.raop_name,
                &advertisement.raop_txt,
            ),
            (
                AIRPLAY_SERVICE,
                &advertisement.airplay_name,
                &advertisement.airplay_txt,
            ),
        ];
        for (service_type, name, properties) in records {
            let info = ServiceInfo::new(
                service_type,
                name,
                &host,
                // The responder fills in the addresses of every interface, which
                // is what a receiver on a machine with several adapters needs.
                "",
                advertisement.port,
                properties.clone(),
            )
            .map(|info| info.enable_addr_auto())
            .map_err(|error| DiscoveryError::Service(error.to_string()))?;
            let fullname = info.get_fullname().to_owned();
            daemon
                .register(info.clone())
                .map_err(|error| DiscoveryError::Register(error.to_string()))?;
            fullnames.push(fullname);
            registered.push(info);
        }
        let (stop, stopped) = mpsc::channel();
        let responder = daemon.clone();
        let worker = std::thread::spawn(move || {
            let mut next_announce = Instant::now() + Duration::from_secs(30);
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(Duration::from_millis(250))
            {
                for event in events.try_iter() {
                    match event {
                        DaemonEvent::Error(error) => {
                            log::warn!("discovery responder error: {error}")
                        }
                        DaemonEvent::NameChange(change) => {
                            log::warn!("discovery name conflict: {change:?}")
                        }
                        DaemonEvent::Announce(name, interface) => {
                            log::info!("discovery announced {name} on {interface}")
                        }
                        DaemonEvent::IpAdd(ip) => log::info!("discovery interface added {ip}"),
                        DaemonEvent::IpDel(ip) => log::warn!("discovery interface removed {ip}"),
                        _ => {}
                    }
                }
                if Instant::now() >= next_announce {
                    // Refresh existing records without unregistering them or
                    // disturbing the receiver/media session. This also recovers
                    // discovery after lost multicast announcements or Wi-Fi roam.
                    for info in &registered {
                        if let Err(error) = responder.register(info.clone()) {
                            log::warn!("discovery refresh failed: {error}");
                        }
                    }
                    next_announce = Instant::now() + Duration::from_secs(30);
                }
            }
        });
        Ok(Self {
            daemon,
            fullnames,
            maintenance: Some((stop, worker)),
        })
    }

    /// Full names of the registered records, for diagnostics.
    pub fn fullnames(&self) -> &[String] {
        &self.fullnames
    }

    /// Withdraws both records and stops the responder.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError::Unregister`] when a withdrawal was refused; the
    /// responder is stopped either way, and dropping the value does the same
    /// without reporting.
    pub fn unregister(mut self) -> Result<(), DiscoveryError> {
        self.stop_maintenance();
        let mut failure = None;
        for fullname in &self.fullnames {
            if let Err(error) = self.daemon.unregister(fullname) {
                failure.get_or_insert_with(|| DiscoveryError::Unregister(error.to_string()));
            }
        }
        if let Err(error) = self.daemon.shutdown() {
            failure.get_or_insert_with(|| DiscoveryError::Unregister(error.to_string()));
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn stop_maintenance(&mut self) {
        if let Some((stop, worker)) = self.maintenance.take() {
            let _ = stop.send(());
            let _ = worker.join();
        }
    }
}

impl Drop for Advertiser {
    /// Withdraws the records without reporting; [`Advertiser::unregister`] is the
    /// version that tells the caller whether that worked.
    fn drop(&mut self) {
        self.stop_maintenance();
        for fullname in &self.fullnames {
            let _ = self.daemon.unregister(fullname);
        }
        let _ = self.daemon.shutdown();
    }
}
