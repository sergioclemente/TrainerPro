import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import MiniHud from "./screens/MiniHud";
import { wireEvents } from "./state";
import { traceEvent } from "./trace";
import "./styles.css";

/** The HUD window loads this same bundle with `?view=hud` (see
    `backend/src/commands/mini_hud.rs`) and renders only the HUD. */
const HUD_VIEW_QUERY = "hud";
const isHudWindow = new URLSearchParams(window.location.search).get("view") === HUD_VIEW_QUERY;

traceEvent("frontend_ready", { view: isHudWindow ? "hud" : "main" });
void wireEvents();
if (isHudWindow) document.documentElement.classList.add("hud-window");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isHudWindow ? <MiniHud /> : <App />}</React.StrictMode>,
);
