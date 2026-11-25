import { isResumable, isRunning, transferPercent } from "./logic";
import type { TransferView } from "./types";

const RADIUS = 17;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/**
 * Arrow dropping onto a tray (receiving) or rising off it (sending), the same
 * drawing and timing as the Android transfer ring. Sending is the receiving
 * drawing mirrored top to bottom.
 */
function FlowArrows({ sending }: { sending: boolean }) {
  const size = 26;
  const cx = size / 2;
  const head = size * 0.2;
  const tip = size * 0.36;
  const shaftTop = tip - size * 0.34;
  const trayY = size * 0.82;
  return (
    <svg className="nc-flow" width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
      <g transform={sending ? `translate(0 ${size}) scale(1 -1)` : undefined}>
        <line
          className="nc-flow-tray"
          x1={size * 0.28}
          y1={trayY}
          x2={size * 0.72}
          y2={trayY}
        />
        <g className="nc-flow-fade">
          <path
            className="nc-flow-arrow"
            d={`M${cx} ${shaftTop} L${cx} ${tip} M${cx - head} ${tip - head} L${cx} ${tip} L${cx + head} ${tip - head}`}
          />
        </g>
      </g>
    </svg>
  );
}

/** Circular percent. Tapping pauses a running transfer and resumes a paused one. */
export function TransferRing({
  transfer,
  onToggle,
}: {
  transfer: TransferView;
  onToggle: () => void;
}) {
  const percent = transferPercent(transfer);
  const running = isRunning(transfer);
  const resumable = isResumable(transfer);
  const tappable = running || resumable;
  const dimmed = running && !transfer.canPause;
  const failed = transfer.state === "failed" || transfer.state === "expired";
  const label =
    transfer.state === "complete"
      ? "✓"
      : failed
        ? "!"
        : transfer.state === "cancelled"
          ? "–"
          : `${percent}%`;
  const title = dimmed
    ? "This device runs an older ClipX and can't pause or resume. You can still cancel."
    : running
      ? "Pause"
      : resumable
        ? "Resume"
        : "";
  const filled = transfer.state === "complete" ? 100 : percent;
  const classes = [
    "nc-ring",
    tappable ? "" : "nc-ring-static",
    dimmed ? "nc-ring-dimmed" : "",
    resumable ? "nc-ring-paused" : "",
    failed ? "nc-ring-failed" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      className={classes}
      onClick={onToggle}
      disabled={!tappable}
      title={title}
      aria-label={title || `Transfer ${transfer.state}`}
    >
      {running && <FlowArrows sending={transfer.direction === "send"} />}
      <svg width="44" height="44" viewBox="0 0 44 44" aria-hidden="true">
        <circle className="nc-ring-track" cx="22" cy="22" r={RADIUS} />
        <circle
          className="nc-ring-arc"
          cx="22"
          cy="22"
          r={RADIUS}
          strokeDasharray={CIRCUMFERENCE}
          strokeDashoffset={CIRCUMFERENCE * (1 - filled / 100)}
          transform="rotate(-90 22 22)"
        />
      </svg>
      <span className="nc-ring-label">{label}</span>
    </button>
  );
}
