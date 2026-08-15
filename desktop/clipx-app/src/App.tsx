import { currentMonitor, getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import "./App.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { showToast, subscribeToast, ToastState } from "./components/toast";
import { Toast } from "./components/Toast.tsx";

type DeviceType = "windows" | "android" | "linux" | "macos" | "ios";
type ConnectionState = "connected" | "connecting" | "disconnected" | "unavailable";
type PairingState = "idle" | "requesting";
type Screen = "devices" | "history";


interface PairedDevice {
  fingerprint: string;
  name: string;
  deviceType: DeviceType;
  connection: ConnectionState;
  ipAddress: string;
  wsPort: number;
  autoConnect: boolean;
}

interface AvailableDevice {
  fingerprint: string;
  name: string;
  deviceType: DeviceType;
  pairing: PairingState;
}

interface ClipItem {
  id: string;
  content: string;
  sourceDevice: string;
  receivedAt: number;
}

interface OwnIdentity {
  name: string;
  fingerprint: string;
  deviceType: string;
  wsPort: number;
  ipAddress: string;
}

const OWN_IDENTITY: OwnIdentity = {
  name: "Clipx Laptop",
  fingerprint: "",
  deviceType: "unknown",
  wsPort: 0,
  ipAddress: "0.0.0.0",
};

const PAIRED_POLL_MS = 4000;

const MOCK_PAIRED: PairedDevice[] = [];

const MOCK_AVAILABLE: AvailableDevice[] = [];

const MOCK_HISTORY: ClipItem[] = [
  { id: "h1", content: "https://github.com/godiegh/clipx", sourceDevice: "samsung SM-G970U", receivedAt: Date.now() - 1000 * 60 * 2 },
  { id: "h2", content: "cargo run --bin clipx-core", sourceDevice: "samsung SM-G970U", receivedAt: Date.now() - 1000 * 60 * 40 },
  { id: "h3", content: "The quick brown fox jumps over the lazy dog. This is a longer clipboard entry to check how wrapping looks.", sourceDevice: "Office Laptop", receivedAt: Date.now() - 1000 * 60 * 60 * 3 },
  { id: "h4", content: "npm run tauri dev", sourceDevice: "Study Mac Mini", receivedAt: Date.now() - 1000 * 60 * 60 * 5 },
  { id: "h5", content: "192.168.0.181:8080", sourceDevice: "Office Laptop", receivedAt: Date.now() - 1000 * 60 * 60 * 8 },
];

const COLLAPSE_THRESHOLD = 4;

/* ---------------------------------------------------------------------- */
/* Custom thin scroll indicator + edge fades (replaces native scrollbar)   */
/* ---------------------------------------------------------------------- */

interface ScrollFadeState {
  canUp: boolean;
  canDown: boolean;
  thumbTop: number;
  thumbHeight: number;
  hasOverflow: boolean;
}

function useScrollFade<T extends HTMLDivElement>() {
  const ref = useRef<T | null>(null);
  const [state, setState] = useState<ScrollFadeState>({
    canUp: false,
    canDown: false,
    thumbTop: 0,
    thumbHeight: 0,
    hasOverflow: false,
  });

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const hasOverflow = scrollHeight - clientHeight > 1;
    const maxScroll = Math.max(scrollHeight - clientHeight, 1);
    const thumbHeight = hasOverflow ? Math.max((clientHeight / scrollHeight) * 100, 10) : 0;
    const thumbTop = hasOverflow ? (scrollTop / maxScroll) * (100 - thumbHeight) : 0;
    setState({
      canUp: scrollTop > 2,
      canDown: hasOverflow && scrollTop < maxScroll - 2,
      thumbTop,
      thumbHeight,
      hasOverflow,
    });
  }, []);

  useEffect(() => {
    measure();
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);

  return { ref, state, onScroll: measure };
}

/**
 * Wraps scrollable content with:
 *  - native scrollbar hidden entirely
 *  - a 2px, non-interactive indicator showing scroll position (only when overflowing)
 *  - top/bottom fade gradients that appear only when there's more content that way
 * `className` controls the sizing behavior per context (see App.css).
 */
function ScrollFade({ className, children }: { className?: string; children: React.ReactNode }) {
  const { ref, state, onScroll } = useScrollFade<HTMLDivElement>();

  return (
    <div className={`scroll-fade-wrap ${className ?? ""}`}>
      <div ref={ref} className="scroll-fade-viewport" onScroll={onScroll}>
        {children}
      </div>
      <div className={`scroll-fade-edge top ${state.canUp ? "visible" : ""}`} />
      <div className={`scroll-fade-edge bottom ${state.canDown ? "visible" : ""}`} />
      {state.hasOverflow && (
        <div
          className="scroll-fade-thumb"
          style={{ top: `${state.thumbTop}%`, height: `${state.thumbHeight}%` }}
        />
      )}
    </div>
  );
}

/* ---------------------------------------------------------------------- */

function DeviceIcon({ type }: { type: DeviceType }) {
  switch (type) {
    case "windows":
      return (
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="3" y="4" width="18" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
          <path d="M8 20h8M12 16v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      );
    case "android":
      return (
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
          <rect x="6" y="3" width="12" height="18" rx="2" stroke="currentColor" strokeWidth="1.5" />
          <line x1="9" y1="19" x2="9" y2="19.1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        </svg>
      );
    default:
      return (
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="1.5" />
        </svg>
      );
  }
}

function connectionLabel(state: ConnectionState) {
  switch (state) {
    case "connected": return "Connected";
    case "connecting": return "Connecting…";
    case "disconnected": return "Connect";
    case "unavailable": return "Not available";
  }
}

function EmptyState({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="empty-state">
      <svg width="34" height="34" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="1.5" />
        <path d="M20 20l-3.2-3.2" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
      <div className="empty-title">{title}</div>
      <div className="empty-hint">{hint}</div>
    </div>
  );
}

function PairedRow({
  device,
  onConnect,
  onOpenDetail,
}: {
  device: PairedDevice;
  onConnect: (fp: string) => void;
  onOpenDetail: (fp: string) => void;
}) {
  const clickable = device.connection === "disconnected";

  return (
    <div className="device-row">
      <button className="device-hit" onClick={() => onOpenDetail(device.fingerprint)}>
        <div className="device-icon">
          <DeviceIcon type={device.deviceType} />
        </div>
        <div className="device-info">
          <div className="device-name">{device.name}</div>
          <div className="device-sub">
            <span className={`status-dot ${device.connection}`} />
            {connectionLabel(device.connection)}
          </div>
        </div>
      </button>
      {clickable && (
        <button className="device-action" onClick={() => onConnect(device.fingerprint)}>
          Connect
        </button>
      )}
    </div>
  );
}

function AvailableRow({ device, onPair }: { device: AvailableDevice; onPair: (fp: string) => void }) {
  const busy = device.pairing === "requesting";

  return (
    <div className="device-row">
      <button className="device-hit">
        <div className="device-icon">
          <DeviceIcon type={device.deviceType} />
        </div>
        <div className="device-info">
          <div className="device-name">{device.name}</div>
          <div className="device-sub">{device.fingerprint.match(/.{1,4}/g)?.slice(0, 7).join("-") ?? device.fingerprint}</div>
        </div>
      </button>
      <button className="device-action" disabled={busy} onClick={() => onPair(device.fingerprint)}>
        {busy ? "Requesting…" : "Pair"}
      </button>
    </div>
  );
}

function DeviceIdentity({ identity, onClose }: { identity: OwnIdentity; onClose: () => void }) {
  const grouped = identity.fingerprint.match(/.{1,4}/g)?.slice(0, 7).join("-") ?? identity.fingerprint;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="identity-panel" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </button>
        <div className="identity-glyph">
          <svg width="36" height="36" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
            <rect x="3" y="4" width="18" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
            <path d="M8 20h8M12 16v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </div>
        <h2>{identity.name}</h2>
        <div className="identity-subtitle">This device</div>

        <div className="identity-grid">
          <div className="identity-cell">
            <label>Type</label>
            <div className="identity-value">{identity.deviceType}</div>
          </div>
          <div className="identity-cell">
            <label>IP Address</label>
            <div className="identity-value">{identity.ipAddress}</div>
          </div>
          <div className="identity-cell">
            <label>WS Port</label>
            <div className="identity-value">{identity.wsPort}</div>
          </div>
        </div>

        <div className="identity-field">
          <label>Fingerprint</label>
          <div className="identity-fingerprint">{grouped}</div>
        </div>
      </div>
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      className={`toggle ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
      role="switch"
      aria-checked={checked}
    >
      <span className="toggle-knob" />
    </button>
  );
}

function DeviceDetail({
  device,
  onClose,
  onDisconnect,
  onForget,
  onToggleAutoConnect,
}: {
  device: PairedDevice;
  onClose: () => void;
  onDisconnect: (fp: string) => void;
  onForget: (fp: string) => void;
  onToggleAutoConnect: (fp: string, value: boolean) => void;
}) {
  const grouped = device.fingerprint.match(/.{1,4}/g)?.slice(0, 7).join("-") ?? device.fingerprint;
  const isConnected = device.connection === "connected" || device.connection === "connecting";
  const deviceAvalable = device.connection != "unavailable";

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="detail-panel" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </button>

        <div className="detail-head">
          <div className="identity-glyph">
            <DeviceIcon type={device.deviceType} />
          </div>
          <h2>{device.name}</h2>
          <div className="identity-subtitle">
            <span className={`status-dot ${device.connection}`} />
            {connectionLabel(device.connection)}
          </div>
        </div>

        {/* If device connection state state says unavalable hence no need to display port + addr */}
        { 
          deviceAvalable &&
          <div className="identity-grid">
            <div className="identity-cell">
              <label>IP Address</label>
              <div className="identity-value">{device.ipAddress}</div>
            </div>
            <div className="identity-cell">
              <label>WS Port</label>
              <div className="identity-value">{device.wsPort}</div>
            </div>
          </div>
        }

        <div className="identity-field">
          <label>Fingerprint</label>
          <div className="identity-fingerprint">{grouped}</div>
        </div>

        <div className="detail-setting">
          <div className="detail-setting-text">
            <div className="detail-setting-title">Auto-connect</div>
            <div className="detail-setting-hint">Connect automatically when this device is available</div>
          </div>
          <Toggle checked={device.autoConnect} onChange={(v) => onToggleAutoConnect(device.fingerprint, v)} />
        </div>

        <div className="detail-actions">
          {isConnected && (
            <button className="detail-tab-btn" onClick={() => onDisconnect(device.fingerprint)}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0M12 2v10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
              Disconnect
            </button>
          )}
          <button className="detail-tab-btn danger" onClick={() => onForget(device.fingerprint)}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M4 7h16M9 7V4.5A1.5 1.5 0 0 1 10.5 3h3A1.5 1.5 0 0 1 15 4.5V7m2 0v13a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V7h10z" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            Forget
          </button>
        </div>
      </div>
    </div>
  );
}

function timeAgo(ts: number) {
  const diffMin = Math.round((Date.now() - ts) / 60000);
  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  return `${Math.round(diffHr / 24)}d ago`;
}

function ClipRow({ item, onCopy, onRemove }: { item: ClipItem; onCopy: (content: string) => void; onRemove: (id: string) => void }) {
  return (
    <div className="clip-row">
      <div className="clip-main">
        <div className="clip-content">{item.content}</div>
        <div className="clip-meta">
          {item.sourceDevice} · {timeAgo(item.receivedAt)}
        </div>
      </div>
      <div className="clip-actions">
        <button className="icon-button" title="Copy to clipboard" onClick={() => onCopy(item.content)}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
            <rect x="8" y="8" width="12" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
            <path d="M16 8V6a1.5 1.5 0 0 0-1.5-1.5h-8A1.5 1.5 0 0 0 5 6v8A1.5 1.5 0 0 0 6.5 16H8" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        </button>
        <button className="icon-button remove" title="Remove" onClick={() => onRemove(item.id)}>
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </div>
  );
}

function CollapsibleList<T>({
  items,
  renderItem,
  keyOf,
  emptyTitle,
  emptyHint,
}: {
  items: T[];
  renderItem: (item: T) => React.ReactNode;
  keyOf: (item: T) => string;
  emptyTitle: string;
  emptyHint: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasOverflow = items.length > COLLAPSE_THRESHOLD;
  const visible = expanded || !hasOverflow ? items : items.slice(0, COLLAPSE_THRESHOLD);

  if (items.length === 0) {
    return <EmptyState title={emptyTitle} hint={emptyHint} />;
  }

  return (
    <div className="collapsible">
      <ScrollFade className="devices-scroll">
        <div className={`device-list ${hasOverflow && !expanded ? "faded" : ""}`}>
          {visible.map((item) => (
            <div key={keyOf(item)}>{renderItem(item)}</div>
          ))}
        </div>
      </ScrollFade>
      {hasOverflow && (
        <button className="collapse-toggle" onClick={() => setExpanded((e) => !e)} title={expanded ? "Show less" : "Show more"}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            style={{ transform: expanded ? "rotate(180deg)" : "none" }}
          >
            <path d="M6 9l6 6 6-6" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      )}
    </div>
  );
}

function App() {
  const appWindow = getCurrentWindow();
  const [screen, setScreen] = useState<Screen>("devices");
  const [paired, setPaired] = useState<PairedDevice[]>(MOCK_PAIRED);
  const [available, setAvailable] = useState<AvailableDevice[]>(MOCK_AVAILABLE);
  const [history, setHistory] = useState<ClipItem[]>(MOCK_HISTORY);
  const [scanning, setScanning] = useState(false);
  const [showIdentity, setShowIdentity] = useState(false);
  const [detailFingerprint, setDetailFingerprint] = useState<string | null>(null);

  const [toast, setToast] = useState<ToastState | null>(null);
  const [ownIdentity, setOwnIdentity] = useState<OwnIdentity>(OWN_IDENTITY);

  useEffect(() => subscribeToast(setToast), []);

  const refreshPaired = useCallback(async () => {
    try {
      const result: PairedDevice[] = await invoke("get_paired_devices");
      setPaired(result);
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  const refreshAvailable = useCallback(async () => {
    try {
      const result: { fingerprint: string; name: string; deviceType: DeviceType }[] =
        await invoke("get_available_devices");
      setAvailable((prev) => {
        const requesting = new Set(prev.filter((d) => d.pairing === "requesting").map((d) => d.fingerprint));
        return result.map((d) => ({ ...d, pairing: requesting.has(d.fingerprint) ? "requesting" : "idle" }));
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  useEffect(() => {
    async function getThisDeviceIdenty() {
      try {
        const result: OwnIdentity = await invoke("get_this_device_identity");
        setOwnIdentity(result);
      } catch (e) {
        showToast(`${e}`, {
          variant: "error",
          onOk: () => appWindow.close(),
          okLabel: "Close App",
        });
      }
    }

    async function positionAppWindow() {
      const monitor = await currentMonitor();
      if (!monitor) return;
      const windowSize = await appWindow.outerSize();
      const workArea = monitor.workArea;
      const margin = 30;
      const x = workArea.position.x + workArea.size.width - windowSize.width - margin;
      const y = workArea.position.y + workArea.size.height - windowSize.height - margin;
      await appWindow.setPosition(new PhysicalPosition(x, y));
      appWindow.show();
      handleScan();
    }

    getThisDeviceIdenty();
    refreshPaired();
    positionAppWindow();

    // Sync mechanism: poll paired devices on an interval. Good enough for now —
    // a push-based version (core broadcasting state changes) can replace this later
    // without changing anything downstream of refreshPaired().
    const interval = setInterval(refreshPaired, PAIRED_POLL_MS);
    return () => clearInterval(interval);
  }, []);

  function handleScan() {
    setScanning(true);
    refreshAvailable().finally(() => setScanning(false));
  }

  async function handleConnect(fingerprint: string) {
    setPaired((prev) => prev.map((d) => (d.fingerprint === fingerprint ? { ...d, connection: "connecting" } : d)));
    try {
      await invoke("connect_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    } finally {
      refreshPaired();
    }
  }

  async function handleDisconnect(fingerprint: string) {
    setDetailFingerprint(null);
    try {
      await invoke("disconnect_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    } finally {
      refreshPaired();
    }
  }

  async function handleForget(fingerprint: string) {
    setDetailFingerprint(null);
    setPaired((prev) => prev.filter((d) => d.fingerprint !== fingerprint)); // optimistic
    try {
      await invoke("forget_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      refreshPaired(); // roll back the optimistic removal on failure
    }
  }

  async function handleToggleAutoConnect(fingerprint: string, value: boolean) {
    setPaired((prev) => prev.map((d) => (d.fingerprint === fingerprint ? { ...d, autoConnect: value } : d)));
    try {
      await invoke("set_auto_connect", { deviceId: fingerprint, autoConnect: value });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      refreshPaired();
    }
  }

  async function handlePair(fingerprint: string) {
    setAvailable((prev) => prev.map((d) => (d.fingerprint === fingerprint ? { ...d, pairing: "requesting" } : d)));
    try {
      await invoke("pair_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      setAvailable((prev) => prev.map((d) => (d.fingerprint === fingerprint ? { ...d, pairing: "idle" } : d)));
    }
    // Note: pairing here only sends the *request* — approval happens elsewhere
    // (handle_approve_pairing on the core side). Once that's wired to a UI action,
    // refreshPaired()/refreshAvailable() here will pick up the result.
  }

  function handleCopy(content: string) {
    navigator.clipboard?.writeText(content).catch(() => { });
  }

  function handleRemoveHistory(id: string) {
    setHistory((prev) => prev.filter((item) => item.id !== id));
  }

  function handleClearAll() {
    setHistory([]);
  }

  const detailDevice = paired.find((d) => d.fingerprint === detailFingerprint) ?? null;

  function renderDevicesScreen() {
    return (
      <ScrollFade className="devices-panel-scroll">
        <div className="devices-grid">
          <section className="device-section">
            <div className="section-header">
              <h2 className="section-title">Paired Devices</h2>
            </div>
            <CollapsibleList
              items={paired}
              keyOf={(d) => d.fingerprint}
              renderItem={(d) => (
                <PairedRow device={d} onConnect={handleConnect} onOpenDetail={setDetailFingerprint} />
              )}
              emptyTitle="No paired devices"
              emptyHint="Pair a nearby device to sync your clipboard."
            />
          </section>

          <section className="device-section">
            <div className="section-header">
              <h2 className="section-title">Available Devices</h2>
              <button className={`scan-button ${scanning ? "spinning" : ""}`} onClick={handleScan} disabled={scanning}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                  <path
                    d="M4 4v6h6M20 20v-6h-6M4.5 15a8 8 0 0 0 14.5 3M19.5 9A8 8 0 0 0 5 6"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                {scanning ? "Scanning…" : "Scan"}
              </button>
            </div>
            {available.length === 0 ? (
              <EmptyState
                title={scanning ? "Scanning nearby…" : "No devices found"}
                hint={scanning ? "Looking for ClipX devices on your network." : "Tap scan to search your network."}
              />
            ) : (
              <ScrollFade className="devices-scroll">
                <div className="device-list">
                  {available.map((d) => (
                    <AvailableRow key={d.fingerprint} device={d} onPair={handlePair} />
                  ))}
                </div>
              </ScrollFade>
            )}
          </section>
        </div>
      </ScrollFade>
    );
  }

  function renderHistoryScreen() {
    return (
      <section className="history-screen">
        <div className="history-content">
          <div className="history-header">
            <div className="history-title-group">
              <h2 className="history-title">Clipboard History</h2>
              {history.length > 0 && <span className="history-count">{history.length}</span>}
            </div>
            {history.length > 0 && (
              <button className="scan-button" onClick={handleClearAll}>
                Clear all
              </button>
            )}
          </div>
          {history.length === 0 ? (
            <EmptyState title="Nothing here yet" hint="Items received from other devices will show up here." />
          ) : (
            <ScrollFade className="history-scroll">
              <div className="device-list">
                {history.map((item) => (
                  <ClipRow key={item.id} item={item} onCopy={handleCopy} onRemove={handleRemoveHistory} />
                ))}
              </div>
            </ScrollFade>
          )}
        </div>
      </section>
    );
  }

  return (
    <>
      <header className="title-bar" data-tauri-drag-region>
        <div className="app-logo">
          <img src="/clipx-icon.png" alt="clipx logo" />
        </div>
        <div className="window-controls">
          <button className="icon-button titlebar-icon" title="Device identity" onClick={() => setShowIdentity(true)}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <rect x="3" y="4" width="18" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
              <path d="M8 20h8M12 16v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
          <button
            className="icon-button titlebar-icon"
            title={screen === "devices" ? "Clipboard history" : "Back to devices"}
            onClick={() => setScreen(screen === "devices" ? "history" : "devices")}
          >
            {screen === "devices" ? (
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <rect x="6" y="3" width="12" height="18" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
                <path d="M9 3h6v2.5H9z" fill="currentColor" />
                <path d="M9 10h6M9 13.5h6M9 17h3.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
              </svg>
            ) : (
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M15 5l-7 7 7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            )}
          </button>
          <button className="closing-button" onClick={() => appWindow.close()}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
        </div>
      </header>

      <main>{screen === "devices" ? renderDevicesScreen() : renderHistoryScreen()}</main>

      {showIdentity && <DeviceIdentity identity={ownIdentity} onClose={() => setShowIdentity(false)} />}

      {detailDevice && (
        <DeviceDetail
          device={detailDevice}
          onClose={() => setDetailFingerprint(null)}
          onDisconnect={handleDisconnect}
          onForget={handleForget}
          onToggleAutoConnect={handleToggleAutoConnect}
        />
      )}

      <Toast toast={toast} />
    </>
  );
}

export default App;