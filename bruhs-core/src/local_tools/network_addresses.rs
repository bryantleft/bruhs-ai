//! This computer's own interface addresses, which the guard refuses as it
//! refuses loopback: a service bound to every interface is reachable on them.

use std::{
    collections::HashSet,
    io,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// How long a read of the interface addresses is trusted. A Wi-Fi, DHCP or
/// VPN change shows up within this.
pub(super) const MAX_AGE: Duration = Duration::from_secs(2);

type Source = Box<dyn Fn() -> io::Result<Vec<IpAddr>> + Send + Sync>;

/// The interface addresses, read again at check time once the copy is older
/// than its `max_age`. A failed read keeps the last good copy.
pub(super) struct OwnAddresses {
    read: Source,
    max_age: Duration,
    cache: Mutex<Option<(Instant, Arc<HashSet<IpAddr>>)>>,
}

impl OwnAddresses {
    pub(super) fn new(
        read: impl Fn() -> io::Result<Vec<IpAddr>> + Send + Sync + 'static,
        max_age: Duration,
    ) -> Self {
        Self {
            read: Box::new(read),
            max_age,
            cache: Mutex::new(None),
        }
    }

    /// This computer's addresses, read with `getifaddrs`.
    pub(super) fn system() -> Self {
        Self::new(interface_addresses, MAX_AGE)
    }

    /// Reads the addresses once now, so a guard with no good copy fails to
    /// start instead of refusing only loopback.
    pub(super) fn primed(self) -> io::Result<Self> {
        let addresses = (self.read)()?;
        let addresses = Arc::new(addresses.into_iter().map(|ip| ip.to_canonical()).collect());
        *self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), addresses));
        Ok(self)
    }

    /// Whether `ip` is one of this computer's addresses (an IPv4-mapped form
    /// counts as its IPv4 address).
    pub(super) fn contains(&self, ip: IpAddr) -> bool {
        self.current().contains(&ip.to_canonical())
    }

    fn current(&self) -> Arc<HashSet<IpAddr>> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fresh = cache
            .as_ref()
            .filter(|(read_at, _)| read_at.elapsed() < self.max_age)
            .map(|(_, addresses)| addresses.clone());
        let addresses = fresh.unwrap_or_else(|| {
            let addresses = match (self.read)() {
                Ok(addresses) => {
                    Arc::new(addresses.into_iter().map(|ip| ip.to_canonical()).collect())
                }
                Err(error) => {
                    tracing::warn!(%error, "network guard couldn't read interface addresses");
                    cache
                        .as_ref()
                        .map(|(_, addresses)| addresses.clone())
                        .unwrap_or_default()
                }
            };
            *cache = Some((Instant::now(), Arc::clone(&addresses)));
            addresses
        });
        drop(cache);
        addresses
    }
}

/// Every address on this computer's interfaces, up or not.
#[cfg(unix)]
pub(super) fn interface_addresses() -> io::Result<Vec<IpAddr>> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `list` is a valid out pointer; on success getifaddrs stores the
    // head of a list that stays valid until the freeifaddrs call below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut addresses = Vec::new();
    let mut cursor = list;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a node of the list getifaddrs returned, not yet freed.
        let entry = unsafe { &*cursor };
        if !entry.ifa_addr.is_null() {
            // SAFETY: a non-null `ifa_addr` points at a sockaddr owned by the list.
            let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
            match family {
                libc::AF_INET => {
                    // SAFETY: an AF_INET sockaddr is a sockaddr_in.
                    let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
                    // `s_addr` holds the address in network byte order.
                    addresses.push(IpAddr::from(address.sin_addr.s_addr.to_ne_bytes()));
                }
                libc::AF_INET6 => {
                    // SAFETY: an AF_INET6 sockaddr is a sockaddr_in6.
                    let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in6>() };
                    addresses.push(IpAddr::from(address.sin6_addr.s6_addr));
                }
                _ => {}
            }
        }
        cursor = entry.ifa_next;
    }
    // SAFETY: `list` came from a successful getifaddrs and is freed once; no
    // reference into it outlives this call.
    unsafe { libc::freeifaddrs(list) };
    Ok(addresses)
}

/// No interface list off Unix: only the loopback and unspecified rule applies.
#[cfg(not(unix))]
pub(super) fn interface_addresses() -> io::Result<Vec<IpAddr>> {
    Ok(Vec::new())
}
