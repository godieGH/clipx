export type ToastVariant = "error" | "info";

export interface ToastOptions {
  variant?: ToastVariant;
  onOk?: () => void;
  onCancel?: () => void;
  okLabel?: string;
  cancelLabel?: string;
  /** ms before auto-dismiss. Only applies if no onOk/onCancel registered. */
  duration?: number;
}

export interface ToastState {
  id: number;
  message: string;
  options: ToastOptions;
}

type Listener = (toast: ToastState | null) => void;

let listener: Listener | null = null;
let counter = 0;
let timer: ReturnType<typeof setTimeout> | null = null;

export function showToast(message: string, options: ToastOptions = {}) {
  if (timer) clearTimeout(timer);
  const toast: ToastState = { id: ++counter, message, options };
  listener?.(toast);

  const hasActions = !!(options.onOk || options.onCancel);
  if (!hasActions) {
    const duration = options.duration ?? 4000;
    timer = setTimeout(() => listener?.(null), duration);
  }
  return toast.id;
}

export function dismissToast() {
  if (timer) clearTimeout(timer);
  listener?.(null);
}

export function subscribeToast(l: Listener) {
  listener = l;
  return () => {
    if (listener === l) listener = null;
  };
} 