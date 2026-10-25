import { useLayoutEffect } from "react";

export function SplashScreen() {
  useLayoutEffect(() => {
    // Apply transparency before the browser paints the splash to avoid an
    // opaque-background flash during startup.
    document.documentElement.classList.add("startup-splash-active");
    return () => document.documentElement.classList.remove("startup-splash-active");
  }, []);

  return (
    <div className="startup-splash" role="status" aria-label="ClipX is starting">
      <div className="startup-splash-card">
        <img className="startup-splash-logo" src="/clipx-icon.png" alt="ClipX" />
        <div className="startup-splash-name">ClipX</div>
        <div className="startup-splash-caption">Getting things ready</div>
        <div className="startup-splash-progress" aria-hidden="true">
          <span />
        </div>
      </div>
    </div>
  );
}
