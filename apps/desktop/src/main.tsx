import { createRoot } from "react-dom/client";
import App from "./App";
import { nativeApi } from "./adapter";
import { DesktopController } from "./controller";
import "./styles.css";

const preview = import.meta.env.DEV && import.meta.env.MODE === "preview";
// Production cannot opt into preview with a URL, localStorage, or runtime failure.
const api = preview
  ? (await import("./preview")).createPreviewApi()
  : nativeApi;
const element = document.getElementById("root");
if (!element) throw new Error("Missing application root");
createRoot(element).render(
  <App controller={new DesktopController(api)} preview={preview} />,
);
