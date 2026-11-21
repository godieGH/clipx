export type DeviceType = "windows" | "android" | "linux" | "macos" | "ios";
export type ConnectionState =
  | "connected"
  | "connecting"
  | "disconnected"
  | "unavailable";
export type PairingState = "idle" | "requesting";
export type Screen = "devices" | "sync" | "history" | "diagnostics";

export type CurrentClipboard = {
  kind: "text" | "rich_text" | "image";
  text: string;
  html?: string;
  width?: number;
  height?: number;
  rgba?: number[];
};

export interface PairedDevice {
  fingerprint: string;
  name: string;
  deviceType: DeviceType;
  connection: ConnectionState;
  ipAddress: string;
  wsPort: number;
  autoConnect: boolean;
}

export interface AvailableDevice {
  fingerprint: string;
  name: string;
  deviceType: DeviceType;
  pairing: PairingState;
}

export interface PickedFile {
  path: string;
  name: string;
  size: number;
  mimeType: string;
}

export interface ClipItem {
  id: string;
  fileId?: string | null;
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

export interface TransferProgress {
  done: number;
  total: number;
  state: string;
  message: string;
}

// Mirrors clipx.PairingEvent — state is the raw proto3 enum ordinal
// (0 = STARTED, 1 = FAILED, 2 = SUCCEEDED), since prost serializes
// proto3 enum fields as plain integers.
export interface PairingEventPayload {
  device_id: string;
  state: 0 | 1 | 2;
  message: string;
}
export const PAIRING_STATE = { STARTED: 0, FAILED: 1, SUCCEEDED: 2 } as const;

export interface OwnIdentity {
  name: string;
  fingerprint: string;
  deviceType: string;
  wsPort: number;
  ipAddress: string;
}

export interface DiagnosticSnapshot {
  appVersion: string;
  coreStatus: string;
  logDirectory: string;
  latestLogFile: string | null;
  recentLogs: string[];
}

export const OWN_IDENTITY: OwnIdentity = {
  name: "Clipx Laptop",
  fingerprint: "",
  deviceType: "unknown",
  wsPort: 0,
  ipAddress: "0.0.0.0",
};

export const COLLAPSE_THRESHOLD = 4;
