import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";

export interface Anchor {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function anchorOf(el: Element | null): Anchor | null {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.left, y: r.top, width: r.width, height: r.height };
}

const EDGE = 8; // min distance from the window edge
const GAP = 12; // space between trigger and bubble (room for the tail)
const EXIT_MS = 150;

interface Placement {
  left?: number;
  top?: number;
  originX: number;
  originY: number;
  tailX: number;
}

export function Popover({
  anchor,
  variant,
  className = "",
  onClose,
  children,
}: {
  anchor: Anchor | null;
  /** bubble = anchored under the trigger with a tail, grow = centered, expands from the trigger */
  variant: "bubble" | "grow";
  className?: string;
  onClose: () => void;
  children: (close: () => void) => ReactNode;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const exitTimer = useRef<number | undefined>(undefined);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const [closing, setClosing] = useState(false);

  // Plays the exit animation, then tells the parent to unmount us.
  const close = useCallback(() => {
    if (exitTimer.current !== undefined) return;
    setClosing(true);
    exitTimer.current = window.setTimeout(onClose, EXIT_MS);
  }, [onClose]);

  useEffect(() => () => window.clearTimeout(exitTimer.current), []);

  // Measure with offset* (unaffected by transforms) before the first paint,
  // so the animation starts from the trigger's position, not a guess.
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;

    const cx = anchor ? anchor.x + anchor.width / 2 : window.innerWidth / 2;
    const cy = anchor ? anchor.y + anchor.height / 2 : window.innerHeight / 2;

    if (variant === "bubble") {
      const w = panel.offsetWidth;
      const left = Math.min(
        Math.max(cx - w / 2, EDGE),
        window.innerWidth - w - EDGE,
      );
      const top = (anchor ? anchor.y + anchor.height : 0) + GAP;
      const tailX = Math.min(Math.max(cx - left, 22), w - 22);
      setPlacement({
        left,
        top,
        originX: tailX,
        originY: cy - top,
        tailX,
      });
    } else {
      setPlacement({
        originX: cx - panel.offsetLeft,
        originY: cy - panel.offsetTop,
        tailX: 0,
      });
    }
  }, [anchor, variant]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    // An anchored bubble would point at the wrong place after a resize.
    const onResize = () => onClose();
    window.addEventListener("keydown", onKey);
    if (variant === "bubble") window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
    };
  }, [close, onClose, variant]);

  const state = closing ? "is-closing" : placement ? "is-open" : "";

  return (
    <div
      className={`modal-overlay popover-overlay popover-overlay--${variant} ${closing ? "is-closing" : ""}`}
      onClick={close}
    >
      <div
        ref={panelRef}
        className={`popover-panel popover-panel--${variant} ${className} ${state}`}
        style={{
          left: placement?.left,
          top: placement?.top,
          transformOrigin: placement
            ? `${placement.originX}px ${placement.originY}px`
            : undefined,
          visibility: placement ? "visible" : "hidden",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {variant === "bubble" && (
          <span className="popover-tail" style={{ left: placement?.tailX }} />
        )}
        {children(close)}
      </div>
    </div>
  );
}