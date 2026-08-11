import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyAppearance, readAppearanceSettings } from "./appearance";
import { startSolarChromeClock } from "./solarChrome";
import "./styles/app.css";
import "./styles/tokens.css";
import "./styles/ui.css";

// Applied before the first render so a dark theme does not flash light first.
applyAppearance(readAppearanceSettings());
const stopSolarChromeClock = startSolarChromeClock();

if (import.meta.hot) import.meta.hot.dispose(stopSolarChromeClock);

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
