import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { installFileDropNavigationGuard } from "./app/input/fileDrop";
import "./app/styles.css";
import { selectClient } from "./client/select";

installFileDropNavigationGuard();

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("Cockpit root element is missing");
}

void selectClient().then((client) => {
  createRoot(rootElement).render(<App client={client} />);
});
