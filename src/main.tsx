import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import "./app/styles.css";
import { selectClient } from "./client/select";

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("Cockpit root element is missing");
}

void selectClient().then((client) => {
  createRoot(rootElement).render(<App client={client} />);
});
