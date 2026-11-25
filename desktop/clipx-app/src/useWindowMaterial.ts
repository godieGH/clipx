import { useEffect } from "react";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The same native Mica backdrop the main window asks for. Windows 11 only:
 * elsewhere, or when Windows refuses, the solid page background stays.
 */
export function useWindowMaterial() {
  useEffect(() => {
    if (!navigator.userAgent.includes("Windows")) return;

    let cancelled = false;
    getCurrentWindow()
      .setEffects({ effects: [Effect.Mica] })
      .then(() => {
        if (!cancelled) {
          document.documentElement.classList.add("window-material-active");
        }
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        document.documentElement.classList.remove("window-material-active");
        console.debug("ClipX: native Mica unavailable; keeping the solid background.", error);
      });

    return () => {
      cancelled = true;
      document.documentElement.classList.remove("window-material-active");
    };
  }, []);
}
