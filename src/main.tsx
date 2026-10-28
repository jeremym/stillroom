import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "@overtone-foundry/app-kit/kit.css";
import App from "./App";
import { applyLook, loadLook } from "@overtone-foundry/app-kit";

// One bundle, two windows: the main window, and Settings (⌘,) opened with ?window=settings.
const SettingsApp = lazy(() => import("./settings/SettingsApp"));
const which = new URLSearchParams(window.location.search).get("window");

applyLook(loadLook());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {which === "settings" ? <Suspense fallback={null}><SettingsApp /></Suspense> : <App />}
  </React.StrictMode>,
);
