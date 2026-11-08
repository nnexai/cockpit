import { invoke } from "@tauri-apps/api/core";
import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { NativeVoicePresentation } from "./app/voice/NativeVoicePresentation";
import { UiIcon } from "./app/UiIcon";
import { installFileDropNavigationGuard } from "./app/input/fileDrop";
import "./app/styles.css";

export type VoiceViewDestination = "workbench" | "inbox";

function VoiceApp() {
  const [destination, setDestination] = useState<VoiceViewDestination | null>(null);
  const [navigationError, setNavigationError] = useState<string | null>(null);
  const navigatingRef = useRef(false);

  const navigate = async (view: VoiceViewDestination) => {
    if (navigatingRef.current) return;
    navigatingRef.current = true;
    setNavigationError(null);
    setDestination(view);
    try {
      await invoke("cockpit_voice_view_close", { view });
    } catch (error) {
      setNavigationError(error instanceof Error ? error.message : String(error));
      setDestination(null);
      navigatingRef.current = false;
    }
  };

  return <div className="app-shell">
    <nav className="view-bar" aria-label="Application views">
      <button type="button" disabled={destination !== null} onClick={() => void navigate("workbench")}><UiIcon name="grid" /> <span className="view-bar-label">Workbench</span></button>
      <button type="button" disabled={destination !== null} onClick={() => void navigate("inbox")}><UiIcon name="comment" /> <span className="view-bar-label">Inbox</span></button>
      <button type="button" aria-current="page">Voice</button>
    </nav>
    {navigationError && <p role="alert">Could not return to the selected view: {navigationError}</p>}
    <div inert={destination !== null} style={{ display: "contents" }}>
      <NativeVoicePresentation />
    </div>
  </div>;
}

installFileDropNavigationGuard();
const rootElement = document.getElementById("root");
if (rootElement === null) throw new Error("Cockpit voice root element is missing");
createRoot(rootElement).render(<VoiceApp />);
