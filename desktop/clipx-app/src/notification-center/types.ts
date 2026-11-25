// Mirrors src-tauri/src/nc_store.rs (serialized in camelCase).

export type NotificationKind =
  | "pairRequest"
  | "confirmCode"
  | "incomingClipboard"
  | "transfer"
  | "info";

export type PromptStatus = "waiting" | "answered" | "expired";

export interface PromptView {
  promptId: string;
  peerName: string;
  /** Pairing code to compare, for confirmCode. */
  code: string;
  /** Preview of what arrived, for incomingClipboard. */
  content: string;
  contentKind: "text" | "richText" | "image" | "file";
  timeoutMs: number;
  expiresAtMs: number;
  status: PromptStatus;
  /** "allowed" | "denied" | "copied" | "ignored" | "handledElsewhere" */
  outcome: string | null;
}

export interface InfoView {
  title: string;
  body: string;
}

export interface TransferView {
  deviceId: string;
  deviceName: string;
  fileId: string;
  fileName: string;
  direction: "download" | "send";
  done: number;
  total: number;
  state: string;
  message: string;
  /** False when the other device is an older build that cannot pause or resume. */
  canPause: boolean;
}

export interface AppNotification {
  id: string;
  kind: NotificationKind;
  unread: boolean;
  createdAtMs: number;
  updatedAtMs: number;
  readAtMs: number | null;
  prompt: PromptView | null;
  info: InfoView | null;
  transfer: TransferView | null;
}

export type PromptDecision = "allow" | "deny" | "copy" | "ignore";
export type TransferAction = "pause" | "resume" | "cancel";
