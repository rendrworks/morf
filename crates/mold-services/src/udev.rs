use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_PROPERTIES: usize = 256;

/// One bounded kernel uevent received from udev's netlink channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UdevEvent {
    pub action: String,
    pub devpath: String,
    pub subsystem: Option<String>,
    pub devname: Option<String>,
    pub properties: BTreeMap<String, String>,
}

/// Native udev monitor failure.
#[derive(Debug)]
pub struct UdevError(String);

impl fmt::Display for UdevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UdevError {}

/// Nonblocking kernel uevent monitor with an optional subsystem filter.
pub struct UdevMonitor {
    socket: OwnedFd,
    subsystem: Option<String>,
}

impl UdevMonitor {
    /// Opens the kernel uevent multicast channel.
    pub fn new(subsystem: Option<String>) -> Result<Self, UdevError> {
        let raw = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_KOBJECT_UEVENT,
            )
        };
        if raw < 0 {
            return Err(last_error("could not open udev monitor"));
        }
        let socket = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as u16;
        address.nl_groups = 1;
        let result = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                (&raw const address).cast(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if result < 0 {
            return Err(last_error("could not bind udev monitor"));
        }
        Ok(Self {
            socket,
            subsystem: subsystem.filter(|value| !value.is_empty()),
        })
    }

    /// Waits up to the supplied timeout for one matching event.
    pub fn next_event(&self, timeout: Duration) -> Result<Option<UdevEvent>, UdevError> {
        let mut descriptor = libc::pollfd {
            fd: self.socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        let ready = unsafe { libc::poll(&raw mut descriptor, 1, milliseconds) };
        if ready < 0 {
            return Err(last_error("could not poll udev monitor"));
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(UdevError("udev monitor reported a socket error".into()));
        }
        if ready == 0 || descriptor.revents & libc::POLLIN == 0 {
            return Ok(None);
        }
        let mut bytes = [0_u8; MAX_EVENT_BYTES];
        let length = unsafe {
            libc::recv(
                self.socket.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                libc::MSG_DONTWAIT | libc::MSG_TRUNC,
            )
        };
        if length < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(UdevError(format!("could not read udev event: {error}")));
        }
        let length =
            usize::try_from(length).map_err(|_| UdevError("invalid udev event size".into()))?;
        if length > bytes.len() {
            return Err(UdevError(format!(
                "udev event exceeds {MAX_EVENT_BYTES} bytes"
            )));
        }
        let event = parse_event(&bytes[..length])?;
        if self
            .subsystem
            .as_deref()
            .is_some_and(|wanted| event.subsystem.as_deref() != Some(wanted))
        {
            return Ok(None);
        }
        Ok(Some(event))
    }
}

fn parse_event(bytes: &[u8]) -> Result<UdevEvent, UdevError> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let header = fields
        .next()
        .ok_or_else(|| UdevError("udev event has no header".into()))?;
    let header = std::str::from_utf8(header)
        .map_err(|_| UdevError("udev event header is not UTF-8".into()))?;
    let (header_action, header_devpath) = header
        .split_once('@')
        .ok_or_else(|| UdevError("udev event header has no device path".into()))?;
    let mut properties = BTreeMap::new();
    for (index, field) in fields.enumerate() {
        if index == MAX_PROPERTIES {
            return Err(UdevError(format!(
                "udev event exceeds {MAX_PROPERTIES} properties"
            )));
        }
        let field = std::str::from_utf8(field)
            .map_err(|_| UdevError("udev event property is not UTF-8".into()))?;
        if let Some((key, value)) = field.split_once('=') {
            properties.insert(key.to_owned(), value.to_owned());
        }
    }
    let action = properties
        .get("ACTION")
        .cloned()
        .unwrap_or_else(|| header_action.to_owned());
    let devpath = properties
        .get("DEVPATH")
        .cloned()
        .unwrap_or_else(|| header_devpath.to_owned());
    Ok(UdevEvent {
        action,
        devpath,
        subsystem: properties.get("SUBSYSTEM").cloned(),
        devname: properties.get("DEVNAME").cloned(),
        properties,
    })
}

fn last_error(context: &str) -> UdevError {
    UdevError(format!("{context}: {}", io::Error::last_os_error()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_uevent_properties() {
        let event = parse_event(
            b"add@/devices/virtual/input/input1\0ACTION=add\0DEVPATH=/devices/virtual/input/input1\0SUBSYSTEM=input\0DEVNAME=input1\0",
        )
        .unwrap();

        assert_eq!(event.action, "add");
        assert_eq!(event.devpath, "/devices/virtual/input/input1");
        assert_eq!(event.subsystem.as_deref(), Some("input"));
        assert_eq!(event.devname.as_deref(), Some("input1"));
    }

    #[test]
    fn rejects_a_header_without_a_device_path() {
        assert!(parse_event(b"add\0ACTION=add\0").is_err());
    }
}
