import type { AppNotification, PromptView, TransferView } from "./types";

export function isWaitingPrompt(n: AppNotification): boolean {
  return n.prompt?.status === "waiting";
}

export function isRunning(t: TransferView): boolean {
  return t.state === "requesting" || t.state === "receiving" || t.state === "sending";
}

export function isResumable(t: TransferView): boolean {
  return t.state === "paused" || t.state === "interrupted";
}

/** Still doing something, or waiting for the user to resume it. */
export function isActive(t: TransferView): boolean {
  return isRunning(t) || isResumable(t) || t.state === "saving";
}

/** A finished download has a saved file to show in its folder. */
export function canReveal(t: TransferView): boolean {
  return t.direction === "download" && t.state === "complete";
}

/** Mirrors the store: a waiting prompt, or a transfer still going (or paused), cannot be marked read. */
export function canMarkRead(n: AppNotification): boolean {
  if (isWaitingPrompt(n)) return false;
  const t = n.transfer;
  return !t || !(isRunning(t) || t.state === "paused" || t.state === "saving");
}

export function transferPercent(t: TransferView): number {
  if (t.total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((t.done / t.total) * 100)));
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1024;
    unit += 1;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** "0:23" until a prompt expires; "0:00" once it has. */
export function countdown(expiresAtMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

export function promptHeading(p: PromptView, kind: AppNotification["kind"]): string {
  if (kind === "pairRequest") return "Pairing request";
  if (kind === "confirmCode") return "Confirm pairing code";
  switch (p.contentKind) {
    case "file":
      return "Incoming file";
    case "image":
      return "Incoming image";
    case "richText":
      return "Incoming formatted text";
    default:
      return "Incoming text";
  }
}

export function promptBody(p: PromptView, kind: AppNotification["kind"]): string {
  const who = p.peerName || "A device";
  if (kind === "pairRequest") return `${who} wants to pair with this device.`;
  if (kind === "confirmCode") return `Make sure this code is the same on ${who}.`;
  return `From ${who}`;
}

/** Labels for the two buttons of a waiting prompt: [accept, refuse]. */
export function promptActions(
  p: PromptView,
  kind: AppNotification["kind"],
): { accept: [string, "allow" | "copy"]; refuse: [string, "deny" | "ignore"] } {
  if (kind === "pairRequest") return { accept: ["Allow", "allow"], refuse: ["Deny", "deny"] };
  if (kind === "confirmCode") return { accept: ["Confirm", "allow"], refuse: ["Reject", "deny"] };
  return {
    accept: [p.contentKind === "file" ? "Download" : "Copy", "copy"],
    refuse: ["Ignore", "ignore"],
  };
}

/** How a prompt that is no longer waiting ended. */
export function outcomeText(p: PromptView): string {
  if (p.status === "expired") return "Expired with no answer";
  switch (p.outcome) {
    case "allowed":
      return "Allowed";
    case "denied":
      return "Denied";
    case "copied":
      return p.contentKind === "file" ? "Download started" : "Copied";
    case "ignored":
      return "Ignored";
    case "handledElsewhere":
      return "Answered on another window";
    default:
      return "Done";
  }
}

export function transferStatusText(t: TransferView): string {
  switch (t.state) {
    case "requesting":
      return t.direction === "send" ? "Waiting" : "Starting";
    case "receiving":
      return "Receiving";
    case "sending":
      return "Sending";
    case "saving":
      return "Saving";
    case "paused":
      return "Paused";
    case "interrupted":
      return "Interrupted";
    case "complete":
      return "Done";
    case "cancelled":
      return "Cancelled";
    case "expired":
      return "Offer expired";
    case "failed":
      return t.message || "Failed";
    default:
      return t.state;
  }
}

export function transferSubtitle(t: TransferView): string {
  const peer = t.deviceName || "another device";
  const who = t.direction === "send" ? `To ${peer}` : `From ${peer}`;
  const size = t.total > 0 ? `${formatBytes(t.done)} of ${formatBytes(t.total)}` : "";
  return [who, transferStatusText(t), size].filter(Boolean).join("  •  ");
}

export function splitByTab(items: AppNotification[]): {
  unread: AppNotification[];
  read: AppNotification[];
} {
  return {
    unread: items.filter((n) => n.unread),
    read: items.filter((n) => !n.unread),
  };
}
