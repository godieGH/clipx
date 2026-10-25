import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { SplashScreen } from "./SplashScreen";

const isSplashWindow = new URLSearchParams(window.location.search).get("splash") === "1";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isSplashWindow ? <SplashScreen /> : <App />}
  </React.StrictMode>,
);
