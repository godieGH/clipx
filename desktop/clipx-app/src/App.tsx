import { currentMonitor, getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import "./App.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { showToast, subscribeToast, ToastState } from "./components/toast";
import { Toast } from "./components/Toast.tsx";
import { listen } from "@tauri-apps/api/event";

type DeviceType = "windows" | "android" | "linux" | "macos" | "ios";
type ConnectionState = "connected" | "connecting" | "disconnected" | "unavailable";
type PairingState = "idle" | "requesting";
type Screen = "devices" | "sync" | "history";
type CurrentClipboard = { kind: "text" | "rich_text" | "image"; text: string; html?: string; width?: number; height?: number; rgba?: number[] };

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

interface PickedFile {
  path: string;
  name: string;
  size: number;
  mimeType: string;
}

interface ClipItem {
  id: string;
  content: string;
  sourceDevice: string;
  receivedAt: number;
  kind: "text" | "rich_text" | "image" | "file";
  html?: string | null;
  fileName?: string | null;
  mimeType?: string | null;
  fileSize?: number;
  fileExpiresAtMs?: number;
  fileDownloaded?: boolean;
  localFilePath?: string | null;
}

// Mirrors clipx.PairingEvent — state is the raw proto3 enum ordinal
// (0 = STARTED, 1 = FAILED, 2 = SUCCEEDED), since prost serializes
// proto3 enum fields as plain integers.
interface PairingEventPayload {
  device_id: string;
  state: 0 | 1 | 2;
  message: string;
}
const PAIRING_STATE = { STARTED: 0, FAILED: 1, SUCCEEDED: 2 } as const;

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
  const isDraggingRef = useRef(false);
  const startYRef = useRef(0);
  const startScrollTopRef = useRef(0);

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

  // Handle dragging the scrollbar thumb
  const handlePointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el) return;

    e.preventDefault();
    e.stopPropagation();

    isDraggingRef.current = true;
    startYRef.current = e.clientY;
    startScrollTopRef.current = el.scrollTop;

    // Capture pointer events even if cursor moves outside the thumb during drag
    e.currentTarget.setPointerCapture(e.pointerId);
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!isDraggingRef.current || !ref.current) return;

    const el = ref.current;
    const { scrollHeight, clientHeight } = el;
    const maxScroll = scrollHeight - clientHeight;
    
    // Calculate ratio of scrollable height to track area
    const thumbHeightPx = Math.max((clientHeight / scrollHeight) * clientHeight, (clientHeight * 10) / 100);
    const trackSpacePx = clientHeight - thumbHeightPx;

    if (trackSpacePx <= 0) return;

    const deltaY = e.clientY - startYRef.current;
    const scrollDelta = (deltaY / trackSpacePx) * maxScroll;

    el.scrollTop = startScrollTopRef.current + scrollDelta;
  }, []);

  const handlePointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (isDraggingRef.current) {
      isDraggingRef.current = false;
      if (e.currentTarget.hasPointerCapture(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    }
  }, []);

  return {
    ref,
    state,
    onScroll: measure,
    thumbProps: {
      onPointerDown: handlePointerDown,
      onPointerMove: handlePointerMove,
      onPointerUp: handlePointerUp,
      onPointerCancel: handlePointerUp,
    },
  };
}

/**
 * Wraps scrollable content with:
 *  - native scrollbar hidden entirely
 *  - a 2px, non-interactive indicator showing scroll position (only when overflowing)
 *  - top/bottom fade gradients that appear only when there's more content that way
 * `className` controls the sizing behavior per context (see App.css).
 */
function ScrollFade({ className, children }: { className?: string; children: React.ReactNode }) {
  const { ref, state, onScroll, thumbProps } = useScrollFade<HTMLDivElement>();

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
          {...thumbProps}
        />
      )}
    </div>
  );
}

/* ---------------------------------------------------------------------- */

function DeviceIcon({ type }: { type: DeviceType }) {
  const desktop = (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect x="3" y="4" width="18" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
      <path d="M8 20h8M12 16v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );

  const mobile = (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect x="6" y="3" width="12" height="18" rx="2" stroke="currentColor" strokeWidth="1.5" />
      <line x1="9" y1="19" x2="9" y2="19.1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );

  const linux = (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Terminal window */}
      <rect x="3" y="4" width="18" height="16" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
      {/* Prompt chevron */}
      <path d="M6.5 9.5 9.5 12l-3 2.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      {/* Cursor line */}
      <line x1="11.5" y1="14.5" x2="15.5" y2="14.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );

  const ios = (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Phone body */}
      <rect x="6.5" y="2.5" width="11" height="19" rx="2.5" stroke="currentColor" strokeWidth="1.5" />
      {/* Dynamic Island */}
      <rect x="10" y="4.3" width="4" height="1.6" rx="0.8" fill="currentColor" />
      {/* Home indicator */}
      <line x1="9.5" y1="19.3" x2="14.5" y2="19.3" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );

  switch (type) {
    case "windows":
      return desktop;
    case "macos":
      return desktop;
    case "linux":
      return linux;
    case "android":
      return mobile;
    case "ios":
      return ios;
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

function timeAgo(ts: number, locale = "en-US"): string {
  const now = new Date();
  const date = new Date(ts);
  const diffSec = Math.floor((now.getTime() - date.getTime()) / 1000);

  if (diffSec < 60) return "just now";

  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "always" });

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return rtf.format(-diffMin, "minute");

  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return rtf.format(-diffHr, "hour");

  const diffDays = Math.floor(diffHr / 24);
  if (diffDays < 7) return rtf.format(-diffDays, "day");

  const diffWeeks = Math.floor(diffDays / 7);
  if (diffWeeks < 4) return rtf.format(-diffWeeks, "week");

  const diffMonths = Math.floor(diffDays / 30);
  if (diffMonths <= 3) return rtf.format(-diffMonths, "month");

  // Fallback to Intl.DateTimeFormat for older dates
  const isSameYear = date.getFullYear() === now.getFullYear();
  const options: Intl.DateTimeFormatOptions = {
    day: "numeric",
    month: "long",
    ...(isSameYear ? {} : { year: "numeric" }),
  };

  return new Intl.DateTimeFormat(locale, options).format(date);
}


function ClipRow({ item, onCopy, onRemove, onDownload, onReveal, transfer, nowMs }: { item: ClipItem; onCopy: (content: string) => void; onRemove: (id: string) => void; onDownload: (id: string) => void; onReveal: (path: string) => void; transfer?: { done: number; total: number; state: string; message: string }; nowMs: number }) {
  const expired = item.kind === "file" && !item.fileDownloaded && !!item.fileExpiresAtMs && nowMs >= item.fileExpiresAtMs;
  return (
    <div className="clip-row">
      <div className="clip-main">
        <div className="clip-content">{item.content}</div>
        <div className="clip-meta">
          {item.sourceDevice} · {timeAgo(item.receivedAt)}
        </div>
        {item.kind === "file" && (
          <div className="file-inline">
            <div>{item.fileName || item.content} · {Math.ceil((item.fileSize || 0) / 1024)} KB {transfer?.message ? `· ${transfer.message}` : ""}</div>
            <div className="file-progress"><span style={{ width: `${item.fileDownloaded ? 100 : transfer?.total ? Math.round((transfer.done / transfer.total) * 100) : 0}%` }} /></div>
          </div>
        )}
      </div>
      <div className="clip-actions">
        {item.kind === "file" && <button className="icon-button" title={expired ? "Offer expired" : "Download file"} onClick={() => onDownload(item.id)} disabled={expired}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
            <path d="M12 3v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>}
        {item.kind === "file" && item.fileDownloaded && item.localFilePath && <button className="icon-button" title="Show in folder" onClick={() => onReveal(item.localFilePath!)}><svg width="15" height="15" viewBox="0 0 24 24" fill="none"><path d="M3.5 7.5A2.5 2.5 0 0 1 6 5h4l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></svg></button>}
        {item.kind !== "file" && <button className="icon-button" title="Copy to clipboard" onClick={() => onCopy(item.content)}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
            <rect x="8" y="8" width="12" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
            <path d="M16 8V6a1.5 1.5 0 0 0-1.5-1.5h-8A1.5 1.5 0 0 0 5 6v8A1.5 1.5 0 0 0 6.5 16H8" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        </button>}
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
  const [paired, setPaired] = useState<PairedDevice[]>([]);
  const [available, setAvailable] = useState<AvailableDevice[]>([]);
  const [history, setHistory] = useState<ClipItem[]>([]);
  const [scanning, setScanning] = useState(false);
  const [showIdentity, setShowIdentity] = useState(false);
  const [showIdentityDisabled, setshowIdentityDisabled] = useState(true);
  const [detailFingerprint, setDetailFingerprint] = useState<string | null>(null);
  const [currentClipboard, setCurrentClipboard] = useState<CurrentClipboard | null>(null);
  const [outgoingFiles, setOutgoingFiles] = useState<PickedFile[]>([]);
  const [isFileDragActive, setIsFileDragActive] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<Record<string, { done: number; total: number; state: string; message: string }>>({});
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [sendingReadyItems, setSendingReadyItems] = useState(false);

  const [toast, setToast] = useState<ToastState | null>(null);
  const [ownIdentity, setOwnIdentity] = useState<OwnIdentity>(OWN_IDENTITY);

  useEffect(() => subscribeToast(setToast), []);

  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    appWindow.onDragDropEvent((event) => {
      if (cancelled || screen !== "sync") return;
      if (event.payload.type === "enter" || event.payload.type === "over") {
        setIsFileDragActive(true);
      } else if (event.payload.type === "leave") {
        setIsFileDragActive(false);
      } else if (event.payload.type === "drop") {
        setIsFileDragActive(false);
        const paths = event.payload.paths ?? [];
        if (!paths.length) return;
        invoke<PickedFile[]>("get_dropped_files", { paths })
          .then((files) => addPickedFiles(files))
          .catch((e) => showToast(`${e}`, { variant: "error" }));
      }
    }).then((off) => { unlisten = off; }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, [appWindow, screen]);

  const refreshPaired = useCallback(async () => {
    try {
      const result: PairedDevice[] = await invoke("get_paired_devices");
      setPaired(result);
      setAvailable((prev) => {
        const pairedIds = new Set(result.map((d) => d.fingerprint));
        return prev.filter(d => !pairedIds.has(d.fingerprint))
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  const refreshAvailable = useCallback(async () => {
    try {
      const result: { fingerprint: string; name: string; deviceType: DeviceType }[] =
        await invoke("get_available_devices");
      setAvailable((prev) => {
        setshowIdentityDisabled(false);
        const requesting = new Set(prev.filter((d) => d.pairing === "requesting").map((d) => d.fingerprint));
        return result.map((d) => ({ ...d, pairing: requesting.has(d.fingerprint) ? "requesting" : "idle" }));
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      setshowIdentityDisabled(true);
    }
  }, []);

  const refreshHistory = useCallback(async () => {
    try {
      const result: ClipItem[] = await invoke("get_clipboard_history", { limit: null });
      setHistory(result);
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  const getThisDeviceIdenty = useCallback(async () => {
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
  }, [])

  useEffect(() => {
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
    getThisDeviceIdenty();
    refreshPaired();
    refreshHistory();
    positionAppWindow();

    // Push-based sync: the core emits these whenever paired/connection or
    // clipboard-history state actually changes, so we just re-fetch on
    // demand instead of guessing an interval. refreshPaired/refreshHistory
    // are unchanged from the polling version — only what triggers them did.
    let cancelled = false;
    const unlistenPromises = [
      listen("devices-changed", () => { if (!cancelled) refreshPaired(); }),
      listen("clipboard-changed", () => { if (!cancelled) refreshHistory(); }),
      // Registered once, for the lifetime of the app — routes by
      // device_id to whichever "Requesting…" button it belongs to,
      // instead of being (re)registered per pair attempt.
      listen<{ entry_id: string; file_id: string; done: number; total: number; state: string; message: string }>("file-transfer", ({ payload }) => {
        if (cancelled) return;
        setDownloadProgress((prev) => ({ ...prev, [payload.entry_id]: payload, [payload.file_id]: payload }));
      }),
      listen<PairingEventPayload>("pairing-event", ({ payload }) => {
        if (cancelled) return;
        if (payload.state === PAIRING_STATE.STARTED) return;

        setAvailable((prev) => prev.map((d) =>
          d.fingerprint === payload.device_id ? { ...d, pairing: "idle" } : d
        ));

        if (payload.state === PAIRING_STATE.FAILED) {
          showToast(payload.message || "Pairing failed", { variant: "error" });
        }
      }),
    ];
    return () => {
      cancelled = true;
      unlistenPromises.forEach((p) => p.then((unlisten) => unlisten()));
    };
  }, []);

  function handleScan() {
    setScanning(true);
    refreshAvailable().finally(() => {
      /// give a little delay to make UI feel real then call refreshAvalable again if any changes
      setTimeout(async () => {
        await refreshAvailable()
        setScanning(false)
      }, 3000)
    });
  }

  async function handleConnect(fingerprint: string) {
    setPaired((prev) => prev.map((d) => (d.fingerprint === fingerprint ? { ...d, connection: "connecting" } : d)));
    try {
      await invoke("connect_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
    // do not refresh wait the poll will pick it
    // since connect device respond doesnt mean the connect it means the connectins req was succecefully made 
    // the poll refresh might pick if done connected
    // this gives it time to show connecting
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
      return;
    }

    // Normal resolution now comes from the "pairing-event" listener
    // registered once in the top-level useEffect (real push from the
    // core, not a timer). Core's own pair session TTL is 90s — this is
    // just a last-resort safety net in case an event is ever missed.
    setTimeout(() => {
      setAvailable((prev) => prev.map((d) =>
        d.fingerprint === fingerprint && d.pairing === "requesting"
          ? { ...d, pairing: "idle" }
          : d
      ));
    }, 95_000);
  }

  function handleCopy(content: string) {
    navigator.clipboard?.writeText(content).catch(() => { });
  }

  function handleRemoveHistory(id: string) {
    setHistory((prev) => prev.filter((item) => item.id !== id));
    invoke("remove_clipboard_entry", { id }).catch((e) => showToast(`${e}`, { variant: "error" }));
  }

  function handleClearAll() {
    setHistory([]);
    invoke("clear_clipboard_history").catch((e) => showToast(`${e}`, { variant: "error" }));
  }

  const connectedCount = paired.filter((d) => d.connection === "connected").length;

  const refreshCurrentClipboard = useCallback(async () => {
    try {
      if (navigator.clipboard?.read) {
        const items = await navigator.clipboard.read();
        const item = items[0];
        if (item) {
          if (item.types.includes("text/html")) {
            const html = await (await item.getType("text/html")).text();
            const text = item.types.includes("text/plain") ? await (await item.getType("text/plain")).text() : html.replace(/<[^>]*>/g, "");
            setCurrentClipboard({ kind: "rich_text", text, html });
            return;
          }
          const imageType = item.types.find((type) => type.startsWith("image/"));
          if (imageType) {
            const blob = await item.getType(imageType);
            const bitmap = await createImageBitmap(blob);
            const canvas = document.createElement("canvas");
            canvas.width = bitmap.width; canvas.height = bitmap.height;
            const ctx = canvas.getContext("2d");
            if (ctx) {
              ctx.drawImage(bitmap, 0, 0);
              const pixels = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
              setCurrentClipboard({ kind: "image", text: `Image ${bitmap.width}×${bitmap.height}`, width: bitmap.width, height: bitmap.height, rgba: Array.from(pixels) });
              bitmap.close();
              return;
            }
            bitmap.close();
          }
        }
      }
      const text = await navigator.clipboard?.readText();
      if (typeof text === "string") setCurrentClipboard(text ? { kind: "text", text } : null);
    } catch {}
  }, []);

  function addPickedFiles(files: PickedFile[]) {
    setOutgoingFiles((prev) => {
      const merged = [...prev];
      for (const file of files) {
        if (!merged.some((existing) => existing.path === file.path)) merged.push(file);
      }
      return merged;
    });
  }

  async function pickFiles() {
    try {
      const files = await invoke<PickedFile[]>("pick_files");
      addPickedFiles(files);
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }

  const sendReadyItems = useCallback(async () => {
    if (connectedCount === 0) { showToast("No connected devices", { variant: "error" }); return; }
    const hasClipboard = !!currentClipboard;
    if (!hasClipboard && outgoingFiles.length === 0) { showToast("Nothing ready to send", { variant: "error" }); return; }
    setSendingReadyItems(true);
    try {
      if (currentClipboard) {
        if (currentClipboard.kind === "rich_text") await invoke("send_rich_text", { text: currentClipboard.text, html: currentClipboard.html || currentClipboard.text });
        else if (currentClipboard.kind === "image" && currentClipboard.rgba && currentClipboard.width && currentClipboard.height) await invoke("send_image", { width: currentClipboard.width, height: currentClipboard.height, rgba: currentClipboard.rgba });
        else await invoke("send_text", { content: currentClipboard.text });
      }
      for (const file of outgoingFiles) {
        await invoke("send_file_path", { name: file.name, mimeType: file.mimeType, path: file.path });
      }
      setOutgoingFiles([]);
      showToast(`${(hasClipboard ? 1 : 0) + outgoingFiles.length} item(s) offered`);
    } catch (e) { showToast(`${e}`, { variant: "error" }); } finally { setSendingReadyItems(false); }
  }, [connectedCount, currentClipboard, outgoingFiles]);

  async function handleDownloadHistoryFile(id: string) {
    try { await invoke("download_clipboard_file", { id }); showToast("File download requested"); }
    catch (e) { showToast(`${e}`, { variant: "error" }); }
  }

  async function handleRevealHistoryFile(path: string) { try { await invoke("reveal_file_location", { path }); } catch (e) { showToast(`${e}`, { variant: "error" }); } }

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

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
    return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
  }

  function renderSyncScreen() {
    const readyCount = (currentClipboard ? 1 : 0) + outgoingFiles.length;
    const clipboardKindLabel = currentClipboard?.kind === "image" ? "Image" : currentClipboard?.kind === "rich_text" ? "Rich text" : "Text";

    return (
      <section className={`sync-screen ${isFileDragActive ? "sync-screen--dragging" : ""}`}>
        <ScrollFade className="sync-scroll">
        <div className="sync-shell">
          <header className="sync-page-header">
            <div>
              <div className="sync-heading-row">
                <div className="sync-device-transfer-icon" aria-hidden="true">
                  <svg viewBox="0 0 36 42" fill="none">
                    <rect x="8" y="2" width="20" height="28" rx="4" stroke="currentColor" strokeWidth="1.8" />
                  </svg>
                </div>
                <div style={{flex: "1", display: "flex", justifyContent: "space-between", alignItems: "center"}}>
                  <div className="sync-heading">Send</div>
                  <div className={`sync-connection-pill ${connectedCount > 0 ? "online" : "offline"}`}>
                    <span className="status-dot" />
                    {connectedCount > 0 ? `${connectedCount} connected` : "No device connected"}
                  </div>
                </div>
              </div>
            </div>
          </header>

          <div className="sync-layout">
            <div className="sync-primary-column">
              <section className="sync-panel clipboard-panel">
                <div className="sync-panel-header sync-clipboard-header">
                  <div className="sync-panel-title">Clipboard</div>
                  <div className="sync-clipboard-actions">
                    {currentClipboard && <span className="sync-kind ready">{clipboardKindLabel}</span>}
                    <button className="sync-refresh sync-refresh--compact" onClick={refreshCurrentClipboard} title="Refresh clipboard"><span className="sync-refresh-icon">↻</span><span>Refresh</span></button>
                  </div>
                </div>
                {currentClipboard ? (
                  <div className="sync-clipboard-preview">
                    <div className="sync-preview-icon">
                      {currentClipboard.kind === "image" ? "▧" : currentClipboard.kind === "rich_text" ? "R" : "T"}
                    </div>
                    <div className="sync-preview-copy">
                      <div className="sync-preview-text">{currentClipboard.text || "Clipboard content"}</div>
                    </div>
                    <button className="sync-selection-remove" title="Don't send clipboard" onClick={() => setCurrentClipboard(null)}>×</button>
                  </div>
                ) : (
                  <button className="sync-clipboard-empty" onClick={refreshCurrentClipboard} title="Read clipboard">
                    <div className="sync-empty-glyph">＋</div>
                    <div>
                      <div className="sync-empty-title">Add clipboard</div>
                      <div className="sync-empty-copy">Tap to read what is currently copied.</div>
                    </div>
                  </button>
                )}
              </section>

              <section className="sync-panel files-panel">
                <div className="sync-panel-header">
                  <div>
                    <div className="sync-panel-title">Files to send</div>
                    <div className="sync-panel-subtitle">Files stay local until you explicitly press Send.</div>
                  </div>
                  {outgoingFiles.length > 0 && <span className="sync-count-chip">{outgoingFiles.length}</span>}
                </div>

                <button className={`sync-dropzone ${isFileDragActive ? "dragging" : ""}`} onClick={pickFiles} title="Choose files">
                  <div className="sync-drop-icon">＋</div>
                  <div className="sync-drop-title">Drop files anywhere</div>
                  <div className="sync-drop-copy">or click to choose files</div>
                </button>

                {outgoingFiles.length > 0 && (
                  <div className="sync-file-list">
                    {outgoingFiles.map((file, index) => (
                      <div className="sync-file-row" key={`${file.name}:${file.size}${/*:${file?.lastModified}*/""}:${index}`}>
                        <div className="sync-file-icon">{file.mimeType.startsWith("image/") ? "▧" : "□"}</div>
                        <div className="sync-file-info">
                          <div className="sync-file-name">{file.name}</div>
                          <div className="sync-file-meta">{file.mimeType} · {formatBytes(file.size)}</div>
                        </div>
                        <button
                          className="sync-file-remove"
                          title={`Remove ${file.name}`}
                          onClick={() => setOutgoingFiles((prev) => prev.filter((_, i) => i !== index))}
                        >
                          ×
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            </div>

            <aside className="sync-aside">
              <div className="sync-summary-panel">
                <div className="sync-summary-label">READY TO SEND</div>
                <div className="sync-summary-number">{readyCount}</div>
                <div className="sync-summary-caption">{readyCount === 1 ? "item" : "items"} in this send</div>

                <div className="sync-summary-breakdown">
                  {currentClipboard && (
                    <div className="sync-breakdown-row"><span>Clipboard</span><strong>1</strong></div>
                  )}
                  {outgoingFiles.length > 0 && (
                    <div className="sync-breakdown-row"><span>Files</span><strong>{outgoingFiles.length}</strong></div>
                  )}
                  {!currentClipboard && outgoingFiles.length === 0 && (
                    <div className="sync-breakdown-empty">Add something to the queue to enable sending.</div>
                  )}
                </div>

                <div className="sync-summary-destination">
                  <span className={`status-dot ${connectedCount > 0 ? "connected" : "disconnected"}`} />
                  <div>
                    <div>{connectedCount > 0 ? "Connected devices" : "No connected devices"}</div>
                    <small>{connectedCount > 0 ? "The bundle will be sent to connected peers." : "Connect a paired device first."}</small>
                  </div>
                </div>

                <button className="sync-send-button" onClick={sendReadyItems} disabled={sendingReadyItems || connectedCount === 0 || readyCount === 0}>
                  <span>{sendingReadyItems ? "Preparing…" : "Send now"}</span>
                  <span className="sync-send-arrow">→</span>
                </button>

                <div className="sync-summary-note">Files are offered as downloadable content and remain available for up to 24 hours.</div>
              </div>
            </aside>
          </div>
        </div>
        </ScrollFade>
      </section>
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
                  <ClipRow key={item.id} item={item} onCopy={handleCopy} onRemove={handleRemoveHistory} onDownload={handleDownloadHistoryFile} onReveal={handleRevealHistoryFile} transfer={downloadProgress[item.id]} nowMs={nowMs} />
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
          <button className={`icon-button titlebar-icon ${showIdentityDisabled ? "button-disable" : ""}`} title="Device identity" onClick={() => {
            getThisDeviceIdenty().finally(() => {
              if (showIdentityDisabled) return;
              setShowIdentity(true);
            });
          }}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <rect x="3" y="4" width="18" height="12" rx="1.5" stroke="currentColor" strokeWidth="1.5" />
              <path d="M8 20h8M12 16v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
          <button className="icon-button titlebar-icon" title="Sync" onClick={() => setScreen("sync")}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round"/>
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
          <button className="closing-button" onClick={() => appWindow.hide()}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
        </div>
      </header>

      <main>{screen === "devices" ? renderDevicesScreen() : screen === "sync" ? renderSyncScreen() : renderHistoryScreen()}</main>

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