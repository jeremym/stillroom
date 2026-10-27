import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/vt323/latin-400.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-400-italic.css";
import "@fontsource/jetbrains-mono/latin-600.css";
import "@fontsource/jetbrains-mono/latin-700.css";
import "./kit/kit.css";
import App from "./App";
import { applyLook, loadLook } from "./kit";

applyLook(loadLook());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
