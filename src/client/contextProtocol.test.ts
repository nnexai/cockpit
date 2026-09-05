import { describe, expect, it } from "vitest";
import { parsePanePresentation } from "./contextProtocol";

describe("ordinary folder presentation", () => {
  const presentation = {
    session_id: "session", pane_id: "pane", terminal_id: "terminal", binding_id: "binding",
    extension: "context", renderer: "context", confidence: "verified_process", reason: "verified viewer",
    roots: [{ root_id: "folder:1:2", kind: "folder", label: "Notes", path: "/notes",
      repository_id: "folder:1:2", checkout_path: "/notes", companion_id: null }],
    default_root_id: "folder:1:2", can_open_context: false, can_open_files: false, files_root_id: null, can_open_review: false, diagnostics: [],
  };

  it("accepts the verified folder renderer returned by the backend", () => {
    expect(parsePanePresentation(presentation).renderer).toBe("context");
    expect(parsePanePresentation(presentation).roots[0].kind).toBe("folder");
  });

  it("does not let a folder root claim companion identity", () => {
    expect(() => parsePanePresentation({ ...presentation,
      roots: [{ ...presentation.roots[0], companion_id: "task-companion" }],
    })).toThrow("Invalid pane presentation");
  });

  it("requires the advertised files root to be a Folder root", () => {
    expect(() => parsePanePresentation({ ...presentation,
      can_open_files: true,
      files_root_id: "missing-folder",
    })).toThrow("Invalid Context root identity");
    expect(() => parsePanePresentation({ ...presentation,
      files_root_id: presentation.roots[0].root_id,
    })).toThrow("Invalid Context root identity");
  });
});
