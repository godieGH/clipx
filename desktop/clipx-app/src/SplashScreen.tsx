import "./App.css";
import { useLayoutEffect } from "react";

export function SplashScreen() {
  useLayoutEffect(() => {
    // Apply transparency before the browser paints the splash to avoid an
    // opaque-background flash during startup.
    document.documentElement.classList.add("startup-splash-active");
    return () => document.documentElement.classList.remove("startup-splash-active");
  }, []);

  return (
    <div className="startup-splash" role="img" aria-label="ClipX">
      <img className="startup-splash-logo" src="/clipx-icon.png" alt="" />
    </div>
  );
}
