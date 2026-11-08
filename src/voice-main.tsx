import { createRoot } from "react-dom/client";
import { VoiceView } from "./app/voice/VoiceView";
import { installFileDropNavigationGuard } from "./app/input/fileDrop";
import "./app/styles.css";

installFileDropNavigationGuard();
const rootElement = document.getElementById("root");
if (rootElement === null) throw new Error("Cockpit voice root element is missing");
createRoot(rootElement).render(<VoiceView />);
