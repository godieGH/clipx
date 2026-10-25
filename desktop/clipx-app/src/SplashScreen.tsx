import { useEffect } from "react";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";

export function SplashScreen() {
  useEffect(() => {
    // Keep the splash readable on every platform. On Windows, reveal the
    // native Mica backdrop only after the OS confirms the effect was applied.
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
        console.debug("ClipX splash: native Mica unavailable; using the solid fallback.", error);
      });

    return () => {
      cancelled = true;
      document.documentElement.classList.remove("window-material-active");
    };
  }, []);

  return (
    <div className="startup-splash" role="status" aria-label="ClipX is starting">
      <img className="startup-splash-logo" src="/clipx-icon.png" alt="ClipX" />
      <div className="startup-splash-name">ClipX</div>
      <div className="startup-splash-caption">Getting things ready</div>
      <div className="startup-splash-progress" aria-hidden="true">
        <span />
      </div>
    </div>
  );
}
