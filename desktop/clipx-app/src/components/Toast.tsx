import { ToastState, dismissToast } from "./toast";

export function Toast({ toast }: { toast: ToastState | null }) {
  if (!toast) return null;
  const { message, options } = toast;
  const { onOk, onCancel, okLabel = "Ok", cancelLabel = "Cancel", variant = "info" } = options;

  return (
    <div className={`toast-container ${variant}`}>
      <div className="content">
        <div className="toast-message">{message}</div>
        {onCancel && (
          <button className="toast-cancel" onClick={() => { onCancel(); dismissToast(); }}>
            {cancelLabel}
          </button>
        )}
        {onOk && (
          <button className="toast-ok" onClick={() => { onOk(); dismissToast(); }}>
            {okLabel}
          </button>
        )}
      </div>
    </div>
  );
} 