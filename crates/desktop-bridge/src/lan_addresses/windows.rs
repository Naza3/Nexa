use super::{Addresses, MAX_NAME_UNITS};
use crate::{BridgeError, LanIpv4Addresses, Result};
use std::{
    collections::BTreeSet,
    mem::size_of,
    net::Ipv4Addr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, ERROR_SUCCESS},
    NetworkManagement::{
        IpHelper::{
            GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_INFO, GAA_FLAG_SKIP_DNS_SERVER,
            GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
            IP_ADAPTER_UNICAST_ADDRESS_LH,
        },
        Ndis::IfOperStatusUp,
    },
    Networking::WinSock::{AF_INET, IpDadStateDeprecated, IpDadStatePreferred, SOCKADDR_IN},
};

const INITIAL_BYTES: usize = 15_000;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ADAPTERS: usize = 256;
const MAX_UNICAST_NODES: usize = 4096;
const MAX_ATTEMPTS: usize = 3;

fn invalid() -> BridgeError {
    BridgeError::new("lan_address_discovery_invalid")
}
fn limit() -> BridgeError {
    BridgeError::new("lan_address_discovery_limit")
}

/// IP Helper owns no allocation: all nodes and strings must remain in this
/// caller-owned buffer. u64 storage provides the SDK's 8-byte alignment.
struct Buffer {
    words: Vec<u64>,
    len: usize,
}
impl Buffer {
    fn new(len: usize) -> Result<Self> {
        if !(size_of::<IP_ADAPTER_ADDRESSES_LH>()..=MAX_BYTES).contains(&len) {
            return Err(limit());
        }
        Ok(Self {
            words: vec![0; len.div_ceil(size_of::<u64>())],
            len,
        })
    }
    fn base(&self) -> *const u8 {
        self.words.as_ptr().cast()
    }
    fn pointer(&mut self) -> *mut IP_ADAPTER_ADDRESSES_LH {
        self.words.as_mut_ptr().cast()
    }
    fn offset(&self, pointer: *const u8, bytes: usize) -> Result<usize> {
        let offset = (pointer as usize)
            .checked_sub(self.base() as usize)
            .ok_or_else(invalid)?;
        if pointer.is_null() || offset.checked_add(bytes).is_none_or(|end| end > self.len) {
            return Err(invalid());
        }
        Ok(offset)
    }
    // SAFETY: T must allow every bit pattern (integers or the SDK's C structs
    // containing only integers/unions/raw pointers). No borrowed reference or
    // pointer derived from untrusted memory is ever dereferenced directly.
    unsafe fn read<T: Copy>(&self, pointer: *const T) -> Result<T> {
        let offset = self.offset(pointer.cast(), size_of::<T>())?;
        // SAFETY: offset proves this initialized allocation contains the full
        // value; unaligned copies avoid assuming alignment of linked nodes.
        Ok(unsafe { self.base().add(offset).cast::<T>().read_unaligned() })
    }
    fn name(&self, pointer: *const u16) -> Result<String> {
        if pointer.is_null() {
            return Ok(String::new());
        }
        let offset = self.offset(pointer.cast(), size_of::<u16>())?;
        let mut units = Vec::with_capacity(MAX_NAME_UNITS);
        for index in 0..=MAX_NAME_UNITS {
            let at = offset + index * size_of::<u16>();
            if at
                .checked_add(size_of::<u16>())
                .is_none_or(|end| end > self.len)
            {
                return Err(invalid());
            }
            // SAFETY: the complete u16 lies within initialized storage.
            let unit = unsafe { self.base().add(at).cast::<u16>().read_unaligned() };
            if unit == 0 {
                return String::from_utf16(&units).map_err(|_| invalid());
            }
            units.push(unit);
        }
        Err(limit())
    }
    fn node<T: Copy>(&self, pointer: *const T) -> Result<()> {
        // SAFETY: a u32 length header permits every bit pattern. SDK node
        // version/length is checked before any complete structure is copied.
        let len = unsafe { self.read(pointer.cast::<u32>())? } as usize;
        if len < size_of::<T>() {
            return Err(invalid());
        }
        self.offset(pointer.cast(), len)?;
        Ok(())
    }
}

fn state_usable(oper_status: i32, dad_state: i32, valid_lifetime: u32) -> bool {
    oper_status == IfOperStatusUp
        && (dad_state == IpDadStatePreferred || dad_state == IpDadStateDeprecated)
        && valid_lifetime != 0
}

fn parse(buffer: &Buffer) -> Result<LanIpv4Addresses> {
    let mut addresses = Addresses::default();
    let mut adapter = buffer.base().cast::<IP_ADAPTER_ADDRESSES_LH>();
    let mut adapters_seen = BTreeSet::new();
    let mut unicast_seen = BTreeSet::new();
    while !adapter.is_null() {
        if !adapters_seen.insert(adapter as usize) {
            return Err(invalid());
        }
        if adapters_seen.len() > MAX_ADAPTERS {
            return Err(limit());
        }
        buffer.node(adapter)?;
        // SAFETY: checked node length and allocation bounds, SDK integers and
        // raw pointers permit all bit patterns. Pointers are validated on use.
        let entry = unsafe { buffer.read(adapter)? };
        let index = unsafe { entry.Anonymous1.Anonymous.IfIndex };
        if entry.OperStatus == IfOperStatusUp && index != 0 {
            let mut name = None;
            let mut unicast = entry.FirstUnicastAddress;
            while !unicast.is_null() {
                if !unicast_seen.insert(unicast as usize) {
                    return Err(invalid());
                }
                if unicast_seen.len() > MAX_UNICAST_NODES {
                    return Err(limit());
                }
                buffer.node(unicast)?;
                // SAFETY: SDK node checked above; no raw pointer dereference.
                let value = unsafe { buffer.read(unicast)? };
                if state_usable(entry.OperStatus, value.DadState, value.ValidLifetime) {
                    let socket_len =
                        usize::try_from(value.Address.iSockaddrLength).map_err(|_| invalid())?;
                    if socket_len < size_of::<u16>() {
                        return Err(invalid());
                    }
                    buffer.offset(value.Address.lpSockaddr.cast(), socket_len)?;
                    // SAFETY: integer family field checked within buffer.
                    let family = unsafe { buffer.read(value.Address.lpSockaddr.cast::<u16>())? };
                    if family == AF_INET {
                        if socket_len < size_of::<SOCKADDR_IN>() {
                            return Err(invalid());
                        }
                        // SAFETY: family, full sockaddr and bounds checked.
                        let socket =
                            unsafe { buffer.read(value.Address.lpSockaddr.cast::<SOCKADDR_IN>())? };
                        // S_addr contains network-order bytes in native u32
                        // storage. Reading its bytes avoids host-endian reversal.
                        let address =
                            Ipv4Addr::from(unsafe { socket.sin_addr.S_un.S_addr }.to_ne_bytes());
                        if address.is_private() {
                            let friendly = match &name {
                                Some(name) => name,
                                None => name.insert(buffer.name(entry.FriendlyName)?),
                            };
                            addresses.insert(index, friendly, address)?;
                        }
                    }
                }
                unicast = value.Next;
            }
        }
        adapter = entry.Next;
    }
    Ok(addresses.finish())
}

pub(super) fn enumerate() -> Result<LanIpv4Addresses> {
    // These assertions also document the buffer allocation's alignment contract.
    const {
        assert!(std::mem::align_of::<IP_ADAPTER_ADDRESSES_LH>() <= std::mem::align_of::<u64>());
    }
    const {
        assert!(
            std::mem::align_of::<IP_ADAPTER_UNICAST_ADDRESS_LH>() <= std::mem::align_of::<u64>()
        );
    }
    let started = Instant::now();
    let mut bytes = INITIAL_BYTES;
    for _ in 0..MAX_ATTEMPTS {
        if started.elapsed() >= Duration::from_secs(3) {
            return Err(BridgeError::new("lan_address_discovery_timeout"));
        }
        let mut buffer = Buffer::new(bytes)?;
        let mut returned = bytes as u32;
        // SAFETY: aligned, writable caller allocation of `returned` bytes; no
        // borrowed references are alive; null Reserved as required by IP Helper.
        // Only IPv4 unicast + friendly names are used. The API's incidental
        // MAC/DNS/gateway metadata is never inspected, retained or returned.
        let result = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                GAA_FLAG_SKIP_ANYCAST
                    | GAA_FLAG_SKIP_MULTICAST
                    | GAA_FLAG_SKIP_DNS_SERVER
                    | GAA_FLAG_SKIP_DNS_INFO,
                std::ptr::null(),
                buffer.pointer(),
                &mut returned,
            )
        };
        match result {
            ERROR_SUCCESS => {
                if returned as usize > buffer.len {
                    return Err(invalid());
                }
                buffer.len = returned as usize;
                return parse(&buffer);
            }
            ERROR_NO_DATA => return Ok(Addresses::default().finish()),
            ERROR_BUFFER_OVERFLOW => {
                bytes = returned as usize;
                if bytes <= buffer.len {
                    return Err(invalid());
                }
            }
            code => {
                let mut error = BridgeError::new("lan_address_discovery_failed");
                error.message.push_str(&format!(" (OS error {code})"));
                return Err(error);
            }
        }
    }
    Err(BridgeError::new("lan_address_discovery_failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Networking::WinSock::{
        IN_ADDR, IN_ADDR_0, IpDadStateDuplicate, IpDadStateInvalid, IpDadStateTentative,
    };
    const UNICAST: usize = 512;
    const SOCKET: usize = 1024;
    const NAME: usize = 1200;
    fn write<T: Copy>(buffer: &mut Buffer, offset: usize, value: T) {
        assert!(offset + size_of::<T>() <= buffer.len);
        // SAFETY: fixtures write only within the initialized owned allocation.
        unsafe {
            buffer
                .words
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<T>()
                .write_unaligned(value);
        }
    }
    fn fixture() -> Buffer {
        let mut buffer = Buffer::new(2048).unwrap();
        let mut adapter = IP_ADAPTER_ADDRESSES_LH::default();
        adapter.Anonymous1.Anonymous.Length = size_of::<IP_ADAPTER_ADDRESSES_LH>() as u32;
        adapter.Anonymous1.Anonymous.IfIndex = 12;
        adapter.OperStatus = IfOperStatusUp;
        adapter.FriendlyName = buffer.base().wrapping_add(NAME).cast_mut().cast();
        adapter.FirstUnicastAddress = buffer.base().wrapping_add(UNICAST).cast_mut().cast();
        let mut unicast = IP_ADAPTER_UNICAST_ADDRESS_LH::default();
        unicast.Anonymous.Anonymous.Length = size_of::<IP_ADAPTER_UNICAST_ADDRESS_LH>() as u32;
        unicast.DadState = IpDadStatePreferred;
        unicast.ValidLifetime = 3600;
        unicast.Address.lpSockaddr = buffer.base().wrapping_add(SOCKET).cast_mut().cast();
        unicast.Address.iSockaddrLength = size_of::<SOCKADDR_IN>() as i32;
        let socket = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from_ne_bytes([192, 168, 20, 30]),
                },
            },
            ..Default::default()
        };
        write(&mut buffer, 0, adapter);
        write(&mut buffer, UNICAST, unicast);
        write(&mut buffer, SOCKET, socket);
        for (index, value) in "测试 VPN\0".encode_utf16().enumerate() {
            write(&mut buffer, NAME + index * 2, value);
        }
        buffer
    }
    #[test]
    fn native_fixture_has_correct_network_order_and_unicode_name() {
        let result = parse(&fixture()).unwrap();
        assert_eq!(result.addresses.len(), 1);
        assert_eq!(result.addresses[0].address, "192.168.20.30");
        assert_eq!(result.addresses[0].interface_name, "测试 VPN");
    }
    #[test]
    fn inactive_unknown_tentative_duplicate_and_expired_are_not_candidates() {
        for status in [0, 2, 3, 4, 5, 6, 7, -1, i32::MAX] {
            assert!(!state_usable(status, IpDadStatePreferred, 3600));
        }
        for dad in [
            IpDadStateInvalid,
            IpDadStateTentative,
            IpDadStateDuplicate,
            -1,
            i32::MAX,
        ] {
            assert!(!state_usable(IfOperStatusUp, dad, 3600));
        }
        assert!(!state_usable(IfOperStatusUp, IpDadStatePreferred, 0));
        assert!(state_usable(IfOperStatusUp, IpDadStateDeprecated, 3600));
    }
    #[test]
    fn linked_cycles_and_short_or_out_of_bounds_nodes_are_rejected() {
        let mut buffer = fixture();
        let mut adapter = unsafe {
            buffer
                .read(buffer.base().cast::<IP_ADAPTER_ADDRESSES_LH>())
                .unwrap()
        };
        adapter.Next = buffer.pointer();
        write(&mut buffer, 0, adapter);
        assert_eq!(
            parse(&buffer).unwrap_err().code,
            "lan_address_discovery_invalid"
        );
        adapter.Next = buffer.base().wrapping_add(buffer.len).cast_mut().cast();
        write(&mut buffer, 0, adapter);
        assert!(parse(&buffer).is_err());
        adapter.Next = std::ptr::null_mut();
        adapter.Anonymous1.Anonymous.Length = 8;
        write(&mut buffer, 0, adapter);
        assert!(parse(&buffer).is_err());
        let mut buffer = fixture();
        let mut unicast = unsafe {
            buffer
                .read(
                    buffer
                        .base()
                        .wrapping_add(UNICAST)
                        .cast::<IP_ADAPTER_UNICAST_ADDRESS_LH>(),
                )
                .unwrap()
        };
        unicast.Next = buffer.base().wrapping_add(UNICAST).cast_mut().cast();
        write(&mut buffer, UNICAST, unicast);
        assert!(parse(&buffer).is_err());
    }
    #[test]
    fn socket_and_name_reads_require_full_bounded_valid_storage() {
        let mut buffer = fixture();
        let mut unicast = unsafe {
            buffer
                .read(
                    buffer
                        .base()
                        .wrapping_add(UNICAST)
                        .cast::<IP_ADAPTER_UNICAST_ADDRESS_LH>(),
                )
                .unwrap()
        };
        unicast.Address.iSockaddrLength = 1;
        write(&mut buffer, UNICAST, unicast);
        assert!(parse(&buffer).is_err());
        let mut buffer = fixture();
        write(&mut buffer, NAME, 0xd800_u16);
        assert!(parse(&buffer).is_err());
        for index in 0..=MAX_NAME_UNITS {
            write(&mut buffer, NAME + index * 2, 65_u16);
        }
        assert_eq!(
            parse(&buffer).unwrap_err().code,
            "lan_address_discovery_limit"
        );
        assert!(
            buffer
                .name(buffer.base().wrapping_add(buffer.len - 1).cast())
                .is_err()
        );
    }
    #[test]
    fn real_windows_read_only_enumeration_does_not_log_local_addresses() {
        let result = enumerate().expect("read-only IP Helper enumeration failed");
        assert!(result.addresses.len() <= super::super::MAX_ADDRESSES);
        for candidate in result.addresses {
            assert!(
                candidate
                    .address
                    .parse::<Ipv4Addr>()
                    .is_ok_and(|address| address.is_private()),
                "non-private address returned"
            );
            assert!(candidate.interface_name.encode_utf16().count() <= MAX_NAME_UNITS);
            assert_ne!(candidate.interface_index, 0);
        }
    }
}
