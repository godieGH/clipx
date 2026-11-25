import { Check, FolderOpen, X } from "lucide-react";
import type { ReactNode } from "react";
import {
  canMarkRead,
  canReveal,
  countdown,
  isResumable,
  isRunning,
  isWaitingPrompt,
  outcomeText,
  promptActions,
  promptBody,
  promptHeading,
  transferSubtitle,
} from "./logic";
import { TransferRing } from "./TransferRing";
import type {
  AppNotification,
  PromptDecision,
  TransferAction,
  TransferView,
} from "./types";

export interface CardHandlers {
  onResolve: (id: string, decision: PromptDecision) => void;
  onTransfer: (id: string, action: TransferAction) => void;
  onMarkRead: (id: string) => void;
  onRemove: (id: string) => void;
  onReveal: (id: string) => void;
}

interface CardProps extends CardHandlers {
  n: AppNotification;
  /** Current time, for countdowns. */
  now: number;
  /** The last problem with this card (a refused action), if any. */
  error?: string;
}

function IconAction({
  title,
  danger,
  onClick,
  children,
}: {
  title: string;
  danger?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      className={`icon-button${danger ? " remove" : ""}`}
      title={title}
      aria-label={title}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/**
 * The last button of a card. Unread: mark it read, once nothing is still
 * running or waiting. Read: take it off the list.
 */
function RowEnd({
  n,
  onMarkRead,
  onRemove,
}: Pick<CardProps, "n" | "onMarkRead" | "onRemove">) {
  if (n.unread) {
    return canMarkRead(n) ? (
      <IconAction title="Mark as read" onClick={() => onMarkRead(n.id)}>
        <Check size={15} />
      </IconAction>
    ) : null;
  }
  return (
    <IconAction title="Remove" danger onClick={() => onRemove(n.id)}>
      <X size={13} />
    </IconAction>
  );
}

export function NotificationCard(props: CardProps) {
  const { n, error } = props;
  return (
    <article
      className={`clip-row nc-card${n.unread ? " nc-unread" : ""}`}
      data-kind={n.kind}
    >
      {n.prompt && <PromptCard {...props} />}
      {n.transfer && <TransferCard {...props} transfer={n.transfer} />}
      {n.info && <InfoCard {...props} />}
      {error && (
        <p className="nc-error" role="alert">
          {error}
        </p>
      )}
    </article>
  );
}

function PromptCard({ n, now, onResolve, onMarkRead, onRemove }: CardProps) {
  const p = n.prompt!;
  const waiting = isWaitingPrompt(n);
  const actions = promptActions(p, n.kind);
  return (
    <>
      <div className="nc-line">
        <div className="nc-text">
          <div className="device-name">{promptHeading(p, n.kind)}</div>
          <div className="clip-meta">{promptBody(p, n.kind)}</div>
        </div>
        {waiting ? (
          <span className="nc-countdown" title="Time left to answer">
            {countdown(p.expiresAtMs, now)}
          </span>
        ) : (
          <RowEnd n={n} onMarkRead={onMarkRead} onRemove={onRemove} />
        )}
      </div>
      {n.kind === "confirmCode" && <p className="nc-code">{p.code}</p>}
      {n.kind === "incomingClipboard" && p.content && (
        <pre className="nc-preview">{p.content}</pre>
      )}
      {waiting ? (
        <div className="nc-actions">
          <button
            className="device-action"
            onClick={() => onResolve(n.id, actions.accept[1])}
          >
            {actions.accept[0]}
          </button>
          <button
            className="word-button"
            onClick={() => onResolve(n.id, actions.refuse[1])}
          >
            {actions.refuse[0]}
          </button>
        </div>
      ) : (
        <div className="clip-meta nc-outcome">{outcomeText(p)}</div>
      )}
    </>
  );
}

function InfoCard({ n, onMarkRead, onRemove }: CardProps) {
  const info = n.info!;
  return (
    <div className="nc-line">
      <div className="nc-text">
        <div className="device-name">{info.title}</div>
        {info.body && <div className="clip-meta">{info.body}</div>}
      </div>
      <RowEnd n={n} onMarkRead={onMarkRead} onRemove={onRemove} />
    </div>
  );
}

function TransferCard({
  n,
  transfer,
  onTransfer,
  onMarkRead,
  onRemove,
  onReveal,
}: CardProps & { transfer: TransferView }) {
  // An interrupted transfer is waiting for the other device, so it is dismissed
  // rather than cancelled; a transfer that is saving can do neither.
  const cancellable = isRunning(transfer) || transfer.state === "paused";
  return (
    <div className="nc-line">
      <TransferRing
        transfer={transfer}
        onToggle={() =>
          onTransfer(n.id, isResumable(transfer) ? "resume" : "pause")
        }
      />
      <div className="nc-text">
        <div className="device-name" title={transfer.fileName}>
          {transfer.fileName || "File"}
        </div>
        <div className="clip-meta">{transferSubtitle(transfer)}</div>
      </div>
      <div className="clip-actions">
        {canReveal(transfer) && (
          <IconAction title="Show in folder" onClick={() => onReveal(n.id)}>
            <FolderOpen size={15} />
          </IconAction>
        )}
        {cancellable && (
          <IconAction
            title="Cancel transfer"
            danger
            onClick={() => onTransfer(n.id, "cancel")}
          >
            <X size={13} />
          </IconAction>
        )}
        <RowEnd n={n} onMarkRead={onMarkRead} onRemove={onRemove} />
      </div>
    </div>
  );
}
