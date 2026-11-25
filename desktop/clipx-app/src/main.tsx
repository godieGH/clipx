import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import NotificationCenter from "./notification-center/NotificationCenter";

// One bundle serves both windows. The notification center is a second window
// (label "notifications") that opens the same page, so the label picks the view.
// Outside Tauri (a plain browser during development) there is no window object
// and the normal app is shown.
function inNotificationCenterWindow(): boolean {
  try {
    return getCurrentWindow().label === "notifications";
  } catch {
    return false;
  }
}
const isNotificationCenter = inNotificationCenterWindow();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isNotificationCenter ? <NotificationCenter /> : <App />}
  </React.StrictMode>,
);
