import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

const rootElement = document.getElementById("root");

if (!rootElement) {
  throw new Error("未找到应用根节点");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
