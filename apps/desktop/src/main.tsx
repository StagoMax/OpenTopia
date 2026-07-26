import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyAppearance, readAppearanceSettings } from "./appearance";
import "./styles/app.css";
import "./styles/tokens.css";
import "./styles/ui.css";

// Applied before the first render so a dark theme does not flash light first.
applyAppearance(readAppearanceSettings());

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
