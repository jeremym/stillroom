import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/vt323/latin-400.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-400-italic.css";
import "@fontsource/jetbrains-mono/latin-600.css";
import "@fontsource/jetbrains-mono/latin-700.css";
import "./kit/kit.css";
import App from "./App";
import { applyLook, loadLook } from "./kit";

// One bundle, two windows: the main window, and Settings (⌘,) opened with ?window=settings.
const SettingsApp = lazy(() => import("./settings/SettingsApp"));
const which = new URLSearchParams(window.location.search).get("window");

applyLook(loadLook());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {which === "settings" ? <Suspense fallback={null}><SettingsApp /></Suspense> : <App />}
  </React.StrictMode>,
);
