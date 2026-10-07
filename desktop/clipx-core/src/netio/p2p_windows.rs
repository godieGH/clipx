//! Windows Wi-Fi Direct link (extra network path only; no ClipX traffic here).
//!
//! * Always hosts a legacy-mode Wi-Fi Direct group, SSID `DIRECT-cw-<fp8>`.
//!   Windows keeps its normal Wi-Fi connection while doing so. Any device
//!   (Android via WifiNetworkSpecifier, another Windows PC) can join it as an
//!   ordinary Wi-Fi client, after which the normal UDP discovery + WebSocket
//!   transport run over that link unchanged.
//! * Only when core currently sees NO peers does it look for another ClipX
//!   host (`DIRECT-cw-*` with a higher fingerprint prefix) and join it.

use crate::device::manager::{DeviceCommands, SeenMode};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use windows::{
    Devices::WiFi::{WiFiAccessStatus, WiFiAdapter, WiFiConnectionStatus, WiFiReconnectionKind},
    Devices::WiFiDirect::{
        WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
    },
    Security::Credentials::PasswordCredential,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
    core::HSTRING,
};

const HOST_PREFIX: &str = "DIRECT-cw-";
const SCAN_EVERY: Duration = Duration::from_secs(20);
const TICK: Duration = Duration::from_millis(500);

/// Same derivation as the Android side: public by nature (the fingerprint is
/// broadcast), it only keeps the link from being joined by accident. Real
/// authentication is still ClipX pairing.
fn ap_pass(host_fp8: &str) -> String {
    let digest = Sha256::digest(format!("clipx-p2p-ap|{host_fp8}").as_bytes()).to_vec();
    hex::encode(digest)[..32].to_string()
}

pub fn spawn(
    shutdown_rx: watch::Receiver<bool>,
    own_fingerprint_hex: String,
    device_tx: mpsc::UnboundedSender<DeviceCommands>,
) -> JoinHandle<()> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || run(shutdown_rx, own_fingerprint_hex, device_tx, rt))
}

fn run(
    shutdown_rx: watch::Receiver<bool>,
    own_fp: String,
    device_tx: mpsc::UnboundedSender<DeviceCommands>,
    rt: tokio::runtime::Handle,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let own8: String = own_fp.chars().take(8).collect();

    let publisher = match start_host(&format!("{HOST_PREFIX}{own8}"), &ap_pass(&own8)) {
        Ok(p) => {
            tracing::info!("Wi-Fi Direct host started: {HOST_PREFIX}{own8}");
            Some(p)
        }
        Err(e) => {
            tracing::warn!("Wi-Fi Direct host unavailable: {e}");
            None
        }
    };

    let mut waited = Duration::ZERO;
    while !*shutdown_rx.borrow() {
        std::thread::sleep(TICK);
        waited += TICK;
        if waited < SCAN_EVERY {
            continue;
        }
        waited = Duration::ZERO;
        if has_peers(&rt, &device_tx) {
            continue;
        }
        match try_join(&own8) {
            Ok(true) => tracing::info!("joined a ClipX Wi-Fi Direct host"),
            Ok(false) => {}
            Err(e) => tracing::debug!("Wi-Fi Direct join attempt failed: {e}"),
        }
    }

    if let Some(p) = publisher {
        let _ = p.Stop();
    }
}

fn start_host(ssid: &str, pass: &str) -> windows::core::Result<WiFiDirectAdvertisementPublisher> {
    let publisher = WiFiDirectAdvertisementPublisher::new()?;
    let adv = publisher.Advertisement()?;
    adv.SetIsAutonomousGroupOwnerEnabled(true)?;
    adv.SetListenStateDiscoverability(WiFiDirectAdvertisementListenStateDiscoverability::Normal)?;
    let legacy = adv.LegacySettings()?;
    legacy.SetIsEnabled(true)?;
    legacy.SetSsid(&HSTRING::from(ssid))?;
    let cred = PasswordCredential::new()?;
    cred.SetPassword(&HSTRING::from(pass))?;
    legacy.SetPassphrase(&cred)?;
    publisher.Start()?;
    Ok(publisher)
}

/// True when core currently sees any peer (LAN or otherwise). Errs on the side
/// of "yes" so we never drop a working network because a query failed.
fn has_peers(rt: &tokio::runtime::Handle, tx: &mpsc::UnboundedSender<DeviceCommands>) -> bool {
    let (reply_to, reply_rx) = oneshot::channel();
    if tx
        .send(DeviceCommands::GetSeen {
            mode: SeenMode::All,
            reply_to,
        })
        .is_err()
    {
        return true;
    }
    rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), reply_rx).await })
        .ok()
        .and_then(|r| r.ok())
        .map(|devices| !devices.is_empty())
        .unwrap_or(true)
}

/// Joins the first `DIRECT-cw-*` host whose fingerprint prefix is higher than
/// ours (the lower side always joins, so two PCs never both wait).
fn try_join(own8: &str) -> windows::core::Result<bool> {
    if WiFiAdapter::RequestAccessAsync()?.join()? != WiFiAccessStatus::Allowed {
        return Ok(false);
    }
    let adapters = WiFiAdapter::FindAllAdaptersAsync()?.join()?;
    if adapters.Size()? == 0 {
        return Ok(false);
    }
    let adapter = adapters.GetAt(0)?;
    adapter.ScanAsync()?.join()?;
    let report = adapter.NetworkReport()?;
    for net in report.AvailableNetworks()? {
        let ssid = net.Ssid()?.to_string();
        let Some(host8) = ssid.strip_prefix(HOST_PREFIX) else {
            continue;
        };
        if host8 <= own8 {
            continue;
        }
        let cred = PasswordCredential::new()?;
        cred.SetPassword(&HSTRING::from(ap_pass(host8)))?;
        let result = adapter
            .ConnectWithPasswordCredentialAsync(&net, WiFiReconnectionKind::Automatic, &cred)?
            .join()?;
        return Ok(result.ConnectionStatus()? == WiFiConnectionStatus::Success);
    }
    Ok(false)
}
