//! Read-only local interface discovery. No runtime, filesystem, socket probe,
//! configuration write, credential initialization, or automatic address choice.
use crate::{DesktopBridge, LanAddressDiscoveryStatus, LanIpv4Addresses, Result};

#[cfg(any(windows, test))]
use crate::{BridgeError, LanIpv4Address};
#[cfg(any(windows, test))]
use std::{collections::BTreeMap, net::Ipv4Addr, sync::Arc, time::Duration};
#[cfg(any(windows, test))]
use tokio::sync::Semaphore;

#[cfg(windows)]
mod windows;

impl DesktopBridge {
    /// Available before initialization and while the service is running. An
    /// observation does not promise reachability or reserve the address.
    pub async fn lan_addresses(&self) -> Result<LanIpv4Addresses> {
        #[cfg(windows)]
        {
            // A timed-out synchronous OS call cannot be cancelled. Its permit
            // remains owned by the worker, so refresh cannot pile up workers.
            static GATE: std::sync::LazyLock<Arc<Semaphore>> =
                std::sync::LazyLock::new(|| Arc::new(Semaphore::new(1)));
            discover_with_deadline(
                Arc::clone(&GATE),
                Duration::from_secs(3),
                windows::enumerate,
            )
            .await
        }
        #[cfg(not(windows))]
        {
            Ok(LanIpv4Addresses {
                status: LanAddressDiscoveryStatus::Unsupported,
                addresses: Vec::new(),
            })
        }
    }
}

#[cfg(any(windows, test))]
async fn discover_with_deadline<F>(
    gate: Arc<Semaphore>,
    deadline: Duration,
    discover: F,
) -> Result<LanIpv4Addresses>
where
    F: FnOnce() -> Result<LanIpv4Addresses> + Send + 'static,
{
    let permit = gate
        .try_acquire_owned()
        .map_err(|_| BridgeError::new("lan_address_discovery_busy"))?;
    let (send, receive) = tokio::sync::oneshot::channel();
    // The synchronous OS API has no cancellation handle. Use one detached
    // worker rather than Tokio's blocking pool, whose shutdown would wait for
    // a stuck OS call even after the UI deadline. It owns no bridge/runtime
    // resources and cannot perform writes. The permit still bounds all calls.
    std::thread::Builder::new()
        .name("nexa-lan-addresses".into())
        .spawn(move || {
            let result = {
                // This inner scope releases admission before the result is
                // sent, including before channel closure during panic unwind.
                let _permit = permit;
                discover()
            };
            let _ = send.send(result);
        })
        .map_err(|_| BridgeError::new("lan_address_discovery_failed"))?;
    tokio::time::timeout(deadline, receive)
        .await
        .map_err(|_| BridgeError::new("lan_address_discovery_timeout"))?
        .map_err(|_| BridgeError::new("lan_address_discovery_failed"))?
}

#[cfg(any(windows, test))]
const MAX_ADDRESSES: usize = 256;
#[cfg(any(windows, test))]
const MAX_NAME_UNITS: usize = 256;

#[cfg(any(windows, test))]
#[derive(Default)]
struct Addresses(BTreeMap<(u32, Ipv4Addr), LanIpv4Address>);

#[cfg(any(windows, test))]
impl Addresses {
    fn insert(&mut self, index: u32, name: &str, address: Ipv4Addr) -> Result<()> {
        // Match the existing listener policy exactly. Do not infer a physical
        // adapter, a trusted network, an allow-list, or a preferred default.
        if !address.is_private() {
            return Ok(());
        }
        let key = (index, address);
        if self.0.contains_key(&key) {
            return Ok(());
        }
        if self.0.len() >= MAX_ADDRESSES || name.encode_utf16().count() > MAX_NAME_UNITS {
            return Err(BridgeError::new("lan_address_discovery_limit"));
        }
        let clean: String = name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let clean = clean.trim();
        self.0.insert(
            key,
            LanIpv4Address {
                interface_index: index,
                interface_name: if clean.is_empty() {
                    format!("网卡 {index}")
                } else {
                    clean.into()
                },
                address: address.to_string(),
            },
        );
        Ok(())
    }

    fn finish(self) -> LanIpv4Addresses {
        LanIpv4Addresses {
            status: if self.0.is_empty() {
                LanAddressDiscoveryStatus::Empty
            } else {
                LanAddressDiscoveryStatus::Available
            },
            addresses: self.0.into_values().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rfc1918_listener_addresses_are_candidates() {
        let mut result = Addresses::default();
        let allowed = [
            "10.0.0.1",
            "10.255.255.254",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.0.1",
        ];
        for address in allowed {
            result
                .insert(1, "Ethernet", address.parse().unwrap())
                .unwrap();
        }
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.1.1",
            "172.15.1.1",
            "172.32.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "8.8.8.8",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            result
                .insert(1, "Ethernet", address.parse().unwrap())
                .unwrap();
        }
        let result = result.finish();
        assert_eq!(result.status, LanAddressDiscoveryStatus::Available);
        assert_eq!(result.addresses.len(), allowed.len());
        for candidate in result.addresses {
            let config = runtime_api::LanApiConfig {
                enabled: true,
                listen: Some(format!("{}:18081", candidate.address).parse().unwrap()),
                allowed_cidrs: vec!["192.168.1.50/32".into()],
            };
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn duplicates_are_per_interface_and_multi_address_vpn_is_preserved() {
        let mut addresses = Addresses::default();
        for (index, name, address) in [
            (2, "Wi-Fi", "192.168.1.20"),
            (1, "Ethernet", "10.0.0.2"),
            (2, "Wi-Fi", "192.168.1.20"),
            (2, "Wi-Fi", "192.168.1.21"),
            (3, "VPN", "10.0.0.2"),
            (4, "vEthernet", "172.16.0.1"),
        ] {
            addresses
                .insert(index, name, address.parse().unwrap())
                .unwrap();
        }
        let result = addresses.finish();
        assert_eq!(result.addresses.len(), 5);
        assert_eq!(result.addresses[0].interface_index, 1);
        assert_eq!(result.addresses[3].interface_name, "VPN");
        assert_eq!(result.addresses[4].interface_name, "vEthernet");
    }

    #[test]
    fn empty_and_bounded_names_and_results() {
        assert_eq!(
            Addresses::default().finish().status,
            LanAddressDiscoveryStatus::Empty
        );
        let mut result = Addresses::default();
        result
            .insert(1, "\n\0", Ipv4Addr::new(10, 0, 0, 1))
            .unwrap();
        assert_eq!(result.0.values().next().unwrap().interface_name, "网卡 1");
        assert_eq!(
            result
                .insert(
                    2,
                    &"x".repeat(MAX_NAME_UNITS + 1),
                    Ipv4Addr::new(10, 0, 0, 1)
                )
                .unwrap_err()
                .code,
            "lan_address_discovery_limit"
        );
        for index in 2..=MAX_ADDRESSES as u32 {
            result
                .insert(index, "Adapter", Ipv4Addr::new(10, 0, 0, 1))
                .unwrap();
        }
        assert_eq!(
            result
                .insert(999, "Adapter", Ipv4Addr::new(10, 0, 0, 1))
                .unwrap_err()
                .code,
            "lan_address_discovery_limit"
        );
    }

    #[tokio::test]
    async fn timeout_retains_single_worker_until_it_finishes() {
        let gate = Arc::new(Semaphore::new(1));
        let (release, wait) = std::sync::mpsc::channel();
        let result =
            discover_with_deadline(Arc::clone(&gate), Duration::from_millis(20), move || {
                let _ = wait.recv();
                Ok(Addresses::default().finish())
            })
            .await;
        assert_eq!(result.unwrap_err().code, "lan_address_discovery_timeout");
        let busy = discover_with_deadline(Arc::clone(&gate), Duration::from_secs(1), || {
            panic!("must not admit second worker")
        })
        .await;
        assert_eq!(busy.unwrap_err().code, "lan_address_discovery_busy");
        release.send(()).unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(2), gate.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert!(
            discover_with_deadline(gate, Duration::from_secs(1), || Ok(
                Addresses::default().finish()
            ))
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn controlled_failure_releases_gate_and_never_contains_interface_data() {
        let gate = Arc::new(Semaphore::new(1));
        let result = discover_with_deadline(Arc::clone(&gate), Duration::from_secs(1), || {
            Err(BridgeError::new("lan_address_discovery_invalid"))
        })
        .await;
        assert_eq!(result.unwrap_err().code, "lan_address_discovery_invalid");
        assert_eq!(gate.available_permits(), 1);
        for code in ["failed", "busy", "timeout", "invalid", "limit"] {
            let error = BridgeError::new(&format!("lan_address_discovery_{code}"));
            assert!(!error.message.contains("192.168."));
            assert!(!error.message.contains("Ethernet"));
            assert!(error.message.contains("手动填写"));
        }
    }

    #[test]
    fn runtime_shutdown_does_not_wait_for_a_timed_out_os_worker() {
        let (release, wait) = std::sync::mpsc::channel();
        let (dropped, observe_drop) = std::sync::mpsc::channel();
        let runtime_thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(discover_with_deadline(
                Arc::new(Semaphore::new(1)),
                Duration::from_millis(20),
                move || {
                    let _ = wait.recv();
                    Ok(Addresses::default().finish())
                },
            ));
            assert_eq!(result.unwrap_err().code, "lan_address_discovery_timeout");
            drop(runtime);
            let _ = dropped.send(());
        });
        let shutdown = observe_drop.recv_timeout(Duration::from_secs(2));
        // Always release the fixture worker, even if this regression fails.
        let _ = release.send(());
        runtime_thread.join().unwrap();
        assert!(
            shutdown.is_ok(),
            "runtime shutdown waited for the OS worker"
        );
    }

    #[tokio::test]
    async fn worker_panic_is_a_controlled_failure_and_releases_admission() {
        let gate = Arc::new(Semaphore::new(1));
        let result = discover_with_deadline(Arc::clone(&gate), Duration::from_secs(1), || {
            panic!("synthetic discovery failure")
        })
        .await;
        assert_eq!(result.unwrap_err().code, "lan_address_discovery_failed");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), gate.acquire())
                .await
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn discovery_never_initializes_or_touches_service_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("not-initialized");
        let bridge = DesktopBridge::new(
            root.clone(),
            temp.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap();
        let result = bridge.lan_addresses().await;
        assert!(!root.exists());
        #[cfg(not(windows))]
        assert_eq!(
            result.unwrap().status,
            LanAddressDiscoveryStatus::Unsupported
        );
        #[cfg(windows)]
        assert!(result.is_ok(), "native read-only discovery failed");
        runtime_api::token::create_private_dir(&root).unwrap();
        std::fs::write(root.join("config.toml"), b"unchanged fixture").unwrap();
        let lock = runtime_cli::instance::InstanceLock::try_acquire(&root)
            .unwrap()
            .unwrap();
        let _ = bridge.lan_addresses().await;
        assert_eq!(
            std::fs::read(root.join("config.toml")).unwrap(),
            b"unchanged fixture"
        );
        assert!(!root.join("secrets").exists());
        drop(lock);
    }
}
