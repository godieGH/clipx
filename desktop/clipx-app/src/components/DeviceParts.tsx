import {
  Circle,
  Monitor,
  Power,
  SearchX,
  Smartphone,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import type {
  AvailableDevice,
  ConnectionState,
  DeviceType,
  OwnIdentity,
  PairedDevice,
} from "../types";
import { groupFingerprint } from "../utils";

export function DeviceIcon({
  type,
  size = 18,
}: {
  type: DeviceType;
  size?: number;
}) {
  switch (type) {
    case "windows":
    case "macos":
      return <Monitor size={size} />;
    case "linux":
      return <Terminal size={size} />;
    case "android":
    case "ios":
      return <Smartphone size={size} />;
    default:
      return <Circle size={size} />;
  }
}

export function connectionLabel(state: ConnectionState) {
  switch (state) {
    case "connected":
      return "Connected";
    case "connecting":
      return "Connecting…";
    case "disconnected":
      return "Connect";
    case "unavailable":
      return "Not available";
  }
}

export function EmptyState({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="empty-state">
      <SearchX size={34} strokeWidth={1.5} />
      <div className="empty-title">{title}</div>
      <div className="empty-hint">{hint}</div>
    </div>
  );
}

export function PairedRow({
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
      <button
        className="device-hit"
        onClick={() => onOpenDetail(device.fingerprint)}
      >
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
        <button
          className="device-action"
          onClick={() => onConnect(device.fingerprint)}
        >
          Connect
        </button>
      )}
    </div>
  );
}

export function AvailableRow({
  device,
  onPair,
}: {
  device: AvailableDevice;
  onPair: (fp: string) => void;
}) {
  const busy = device.pairing === "requesting";

  return (
    <div className="device-row">
      <button className="device-hit">
        <div className="device-icon">
          <DeviceIcon type={device.deviceType} />
        </div>
        <div className="device-info">
          <div className="device-name">{device.name}</div>
          <div className="device-sub">
            {groupFingerprint(device.fingerprint)}
          </div>
        </div>
      </button>
      <button
        className="device-action"
        disabled={busy}
        onClick={() => onPair(device.fingerprint)}
      >
        {busy ? "Requesting…" : "Pair"}
      </button>
    </div>
  );
}

export function DeviceIdentity({
  identity,
  onClose,
}: {
  identity: OwnIdentity;
  onClose: () => void;
}) {
  const grouped = groupFingerprint(identity.fingerprint);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="identity-panel" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>
          <X size={14} />
        </button>
        <div className="identity-glyph">
          <Monitor size={36} strokeWidth={1.5} />
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

export function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
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

export function DeviceDetail({
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
  const grouped = groupFingerprint(device.fingerprint);
  const isConnected =
    device.connection === "connected" || device.connection === "connecting";
  const deviceAvailable = device.connection !== "unavailable";

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="detail-panel" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>
          <X size={14} />
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

        {/* If the device is unavailable there's no point showing port + addr */}
        {deviceAvailable && (
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
        )}

        <div className="identity-field">
          <label>Fingerprint</label>
          <div className="identity-fingerprint">{grouped}</div>
        </div>

        <div className="detail-setting">
          <div className="detail-setting-text">
            <div className="detail-setting-title">Auto-connect</div>
            <div className="detail-setting-hint">
              Connect automatically when this device is available
            </div>
          </div>
          <Toggle
            checked={device.autoConnect}
            onChange={(v) => onToggleAutoConnect(device.fingerprint, v)}
          />
        </div>

        <div className="detail-actions">
          {isConnected && (
            <button
              className="detail-tab-btn"
              onClick={() => onDisconnect(device.fingerprint)}
            >
              <Power size={15} />
              Disconnect
            </button>
          )}
          <button
            className="detail-tab-btn danger"
            onClick={() => onForget(device.fingerprint)}
          >
            <Trash2 size={15} />
            Forget
          </button>
        </div>
      </div>
    </div>
  );
}
