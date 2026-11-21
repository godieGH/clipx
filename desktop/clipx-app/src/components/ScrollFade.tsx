import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";

interface ScrollFadeState {
  canUp: boolean;
  canDown: boolean;
  thumbTop: number;
  thumbHeight: number;
  hasOverflow: boolean;
}

function useScrollFade<T extends HTMLDivElement>() {
  const ref = useRef<T | null>(null);
  const isDraggingRef = useRef(false);
  const startYRef = useRef(0);
  const startScrollTopRef = useRef(0);

  const [state, setState] = useState<ScrollFadeState>({
    canUp: false,
    canDown: false,
    thumbTop: 0,
    thumbHeight: 0,
    hasOverflow: false,
  });

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const hasOverflow = scrollHeight - clientHeight > 1;
    const maxScroll = Math.max(scrollHeight - clientHeight, 1);
    const thumbHeight = hasOverflow
      ? Math.max((clientHeight / scrollHeight) * 100, 10)
      : 0;
    const thumbTop = hasOverflow
      ? (scrollTop / maxScroll) * (100 - thumbHeight)
      : 0;
    setState({
      canUp: scrollTop > 2,
      canDown: hasOverflow && scrollTop < maxScroll - 2,
      thumbTop,
      thumbHeight,
      hasOverflow,
    });
  }, []);

  useEffect(() => {
    measure();
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);

  // Handle dragging the scrollbar thumb
  const handlePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      const el = ref.current;
      if (!el) return;

      e.preventDefault();
      e.stopPropagation();

      isDraggingRef.current = true;
      startYRef.current = e.clientY;
      startScrollTopRef.current = el.scrollTop;

      // Capture pointer events even if cursor moves outside the thumb during drag
      e.currentTarget.setPointerCapture(e.pointerId);
    },
    [],
  );

  const handlePointerMove = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!isDraggingRef.current || !ref.current) return;

      const el = ref.current;
      const { scrollHeight, clientHeight } = el;
      const maxScroll = scrollHeight - clientHeight;

      // Calculate ratio of scrollable height to track area
      const thumbHeightPx = Math.max(
        (clientHeight / scrollHeight) * clientHeight,
        (clientHeight * 10) / 100,
      );
      const trackSpacePx = clientHeight - thumbHeightPx;

      if (trackSpacePx <= 0) return;

      const deltaY = e.clientY - startYRef.current;
      const scrollDelta = (deltaY / trackSpacePx) * maxScroll;

      el.scrollTop = startScrollTopRef.current + scrollDelta;
    },
    [],
  );

  const handlePointerUp = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (isDraggingRef.current) {
        isDraggingRef.current = false;
        if (e.currentTarget.hasPointerCapture(e.pointerId)) {
          e.currentTarget.releasePointerCapture(e.pointerId);
        }
      }
    },
    [],
  );

  return {
    ref,
    state,
    onScroll: measure,
    thumbProps: {
      onPointerDown: handlePointerDown,
      onPointerMove: handlePointerMove,
      onPointerUp: handlePointerUp,
      onPointerCancel: handlePointerUp,
    },
  };
}

/**
 * Wraps scrollable content with:
 *  - native scrollbar hidden entirely
 *  - a thin indicator showing scroll position (only when overflowing)
 *  - top/bottom fade gradients that appear only when there's more content that way
 * `className` controls the sizing behavior per context (see App.css).
 */
export function ScrollFade({
  className,
  children,
  autoScrollKey,
}: {
  className?: string;
  children: ReactNode;
  autoScrollKey?: string;
}) {
  const { ref, state, onScroll, thumbProps } = useScrollFade<HTMLDivElement>();

  useEffect(() => {
    if (autoScrollKey === undefined) return;
    const el = ref.current;
    if (!el) return;
    if (el.scrollHeight - el.clientHeight > 1) {
      el.scrollTop = el.scrollHeight;
    }
    // Update the custom scrollbar indicator after the log content changes.
    onScroll();
  }, [autoScrollKey, onScroll]);

  return (
    <div className={`scroll-fade-wrap ${className ?? ""}`}>
      <div ref={ref} className="scroll-fade-viewport" onScroll={onScroll}>
        {children}
      </div>
      <div className={`scroll-fade-edge top ${state.canUp ? "visible" : ""}`} />
      <div
        className={`scroll-fade-edge bottom ${state.canDown ? "visible" : ""}`}
      />
      {state.hasOverflow && (
        <div
          className="scroll-fade-thumb"
          style={{ top: `${state.thumbTop}%`, height: `${state.thumbHeight}%` }}
          {...thumbProps}
        />
      )}
    </div>
  );
}
