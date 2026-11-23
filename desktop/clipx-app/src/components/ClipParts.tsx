import { useState } from "react";
import type { ReactNode } from "react";
import { ChevronDown, Copy, Download, FolderOpen, X } from "lucide-react";
import { COLLAPSE_THRESHOLD } from "../types";
import type { ClipItem, TransferProgress } from "../types";
import { formatDataSize, timeAgo } from "../utils";
import { ScrollFade } from "./ScrollFade";
import { EmptyState } from "./DeviceParts";

export function ClipRow({
  item,
  onCopy,
  onRemove,
  onDownload,
  onReveal,
  transfer,
  nowMs,
}: {
  item: ClipItem;
  onCopy: (id: string) => void;
  onRemove: (id: string) => void;
  onDownload: (id: string) => void;
  onReveal: (path: string) => void;
  transfer?: TransferProgress;
  nowMs: number;
}) {
  const expired =
    item.kind === "file" &&
    !item.fileDownloaded &&
    !!item.fileExpiresAtMs &&
    nowMs >= item.fileExpiresAtMs;
  const transferActive =
    transfer?.state === "requesting" ||
    transfer?.state === "receiving" ||
    transfer?.state === "sending" ||
    transfer?.state === "saving";
  const transferPercent = transfer?.total
    ? Math.round((transfer.done / transfer.total) * 100)
    : 0;
  const disabled = expired || item.fileDownloaded || transferActive;
  const status =
    transfer?.state === "requesting"
      ? "Starting download…"
      : transfer?.state === "receiving"
        ? `Downloading · ${transferPercent}%`
        : transfer?.state === "saving"
          ? "Saving file…"
          : transfer?.state === "complete" || item.fileDownloaded
            ? "Downloaded"
            : transfer?.state === "failed"
              ? "Download failed · Retry"
              : expired
                ? "Offer expired"
                : "Available for download for up to 24 hours";

  return (
    <div className="clip-row">
      <div className="clip-main">
        <div className="clip-top">
          <div className="clip-text">
            <div className="clip-content">{item.content}</div>
            <div className="clip-meta">
              {item.sourceDevice} · {timeAgo(item.receivedAt)}
            </div>
          </div>
          <div className="clip-actions">
            {item.kind === "file" && (
              <button
                className="icon-button"
                title={disabled ? status : "Download file"}
                onClick={() => onDownload(item.id)}
                disabled={disabled}
              >
                <Download size={15} />
              </button>
            )}
            {item.kind === "file" &&
              item.fileDownloaded &&
              item.localFilePath && (
                <button
                  className="icon-button"
                  title="Show in folder"
                  onClick={() => onReveal(item.localFilePath!)}
                >
                  <FolderOpen size={15} />
                </button>
              )}
            {item.kind !== "file" && (
              <button
                className="icon-button"
                title="Copy to clipboard"
                onClick={() => onCopy(item.id)}
              >
                <Copy size={15} />
              </button>
            )}
            <button
              className="icon-button remove"
              title="Remove"
              onClick={() => onRemove(item.id)}
            >
              <X size={13} />
            </button>
          </div>
        </div>

        {item.kind === "file" && (
          <div className="file-inline">
            <div
              className="file-progress"
              aria-label={status}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={
                transfer?.total
                  ? transferPercent
                  : item.fileDownloaded
                    ? 100
                    : undefined
              }
            >
              <span
                style={{
                  width: `${item.fileDownloaded ? 100 : transfer?.total ? transferPercent : 0}%`,
                }}
              />
            </div>
            <div className="file-status">
              <div className="file-status-text">{status}</div>
              <div className="file-size">
                {formatDataSize(item.fileSize ?? 0)}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export function CollapsibleList<T>({
  items,
  renderItem,
  keyOf,
  emptyTitle,
  emptyHint,
}: {
  items: T[];
  renderItem: (item: T) => ReactNode;
  keyOf: (item: T) => string;
  emptyTitle: string;
  emptyHint: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasOverflow = items.length > COLLAPSE_THRESHOLD;
  const visible =
    expanded || !hasOverflow ? items : items.slice(0, COLLAPSE_THRESHOLD);

  if (items.length === 0) {
    return <EmptyState title={emptyTitle} hint={emptyHint} />;
  }

  return (
    <div className="collapsible">
      <ScrollFade className="devices-scroll">
        <div
          className={`device-list ${hasOverflow && !expanded ? "faded" : ""}`}
        >
          {visible.map((item) => (
            <div key={keyOf(item)}>{renderItem(item)}</div>
          ))}
        </div>
      </ScrollFade>
      {hasOverflow && (
        <button
          className="collapse-toggle"
          onClick={() => setExpanded((e) => !e)}
          title={expanded ? "Show less" : "Show more"}
        >
          <ChevronDown
            size={14}
            style={{ transform: expanded ? "rotate(180deg)" : "none" }}
          />
        </button>
      )}
    </div>
  );
}
