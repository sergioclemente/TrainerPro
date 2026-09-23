import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { wireEvents } from "./state";
import { traceEvent } from "./trace";
import "./styles.css";

traceEvent("frontend_ready");
void wireEvents();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
