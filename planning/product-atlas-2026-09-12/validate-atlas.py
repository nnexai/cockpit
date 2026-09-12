import hashlib, json, re
from pathlib import Path

ROOT = Path(__file__).parent
BASELINE = "1d09b5c1ecd69c427be3bf6ab60933fa9eccfed5"
CATS = {
 "startup": ("session-lifecycle", ["navigation-layout"]), "spaces": ("navigation-layout", ["session-lifecycle"]),
 "tabs": ("navigation-layout", ["terminal-rendering"]), "menus": ("navigation-layout", ["input-focus"]),
 "terminal.focus": ("input-focus", ["terminal-rendering"]), "terminal.scroll": ("terminal-rendering", ["input-focus"]),
 "loading": ("errors-recovery", ["terminal-rendering", "session-lifecycle"]), "browser-native": ("terminal-rendering", ["extension-panes"]),
 "browser.lifecycle": ("extension-panes", ["session-lifecycle", "provenance-safety"]), "browser.popup": ("extension-panes", ["errors-recovery"]),
 "browser.annotation": ("extension-panes", ["input-focus"]), "browser.capture": ("extension-panes", ["provenance-safety"]),
 "browser.stale": ("errors-recovery", ["extension-panes"]), "browser.feedback": ("extension-panes", ["session-lifecycle"]),
 "browser.delivery": ("provenance-safety", ["extension-panes", "input-focus"]), "browser.failure": ("errors-recovery", ["extension-panes"]),
 "setup.repository": ("session-lifecycle", ["provenance-safety"]), "setup.recovery": ("errors-recovery", ["session-lifecycle", "provenance-safety"]),
 "context.files": ("context-files", ["input-focus", "review-comments"]), "context.sources": ("providers-sources", ["context-files", "provenance-safety"]),
 "review.local": ("review-comments", ["context-files", "provenance-safety"]),
}

def categories(sid):
    for key, val in CATS.items():
        if sid.startswith(key): return val
    return ("errors-recovery", [])

def artifact(a, scenario_dir):
    if isinstance(a, str): a = {"path": a}
    else: a = dict(a)
    p = a.get("path", "")
    prov = a.get("provenance", "")
    low = (p + " " + prov + " " + a.get("kind", "")).lower()
    copied_png = p.endswith(".png") and (scenario_dir / Path(p).name).exists()
    if copied_png:
        actual = scenario_dir / Path(p).name
        a["path"] = p
        a["kind"] = "design-reference"
        a["evidenceClass"] = "mock"
        a["capturedAt"] = a.get("originalEvidenceDate", "2026-09-06")
        a["sourceCommit"] = a.get("originalCommit", "unknown")
        a["provenance"] = a.get("provenance", "Copied UI-polish design reference; not runtime proof.")
        a["sha256"] = hashlib.sha256(actual.read_bytes()).hexdigest()
        return a
    historical = ("/tmp/" in p or "checkpoint" in low or "handoff" in low or "daily-use-2026-09-06" in low)
    if historical:
        a["evidenceClass"] = "historical"
        a["capturedAt"] = "2026-09-08" if ("be68fc8" in low or "handoff" in low or "/tmp/" in p) else "2026-09-06"
        a["sourceCommit"] = "be68fc8" if ("be68fc8" in low or "handoff" in low or "/tmp/" in p) else "7084603beeb5261fbefd7b658496767a1d716983"
    elif p.startswith("src/") or p.startswith("browser-extension/") or p.startswith("scripts/") or p.startswith("crates/"):
        a["evidenceClass"] = "static"
        a["capturedAt"] = "2026-09-12"
        a["sourceCommit"] = BASELINE
    else:
        a["evidenceClass"] = a.get("evidenceClass", "static")
        a["capturedAt"] = a.get("capturedAt", "2026-09-12")
        a["sourceCommit"] = a.get("sourceCommit", BASELINE)
    a.setdefault("provenance", "Recorded atlas evidence; see authority for source identity.")
    a.setdefault("sha256", None)
    return a

def normalize(s, d):
    sid = s["id"]
    primary, secondary = categories(sid)
    s["primaryCategory"], s["secondaryCategories"] = primary, secondary
    if sid in ("setup.repository-artifact-to-workspace", "setup.recovery-and-teardown", "context.sources-import-and-snapshot"):
        s["implemented"] = True
        additions = {
            "setup.repository-artifact-to-workspace": ["src/app/projects/SetupDialog.tsx", "planning/next-level/execution/run-20260904T214621Z/WORKFLOW_ACCEPTANCE.md"],
            "setup.recovery-and-teardown": ["src/app/projects/TeardownDialog.tsx", "src/app/projects/TeardownRecoveryPanel.tsx", "planning/next-level/execution/run-20260904T214621Z/WORKFLOW_ACCEPTANCE.md"],
            "context.sources-import-and-snapshot": ["src/app/projects/SourceImport.tsx", "src/app/projects/SnapshotImport.tsx", "planning/next-level/execution/run-20260904T214621Z/FEATURE_ACCEPTANCE.md"],
        }
        s["authority"] = list(dict.fromkeys(s.get("authority", []) + additions[sid]))
    if sid.startswith("browser."):
        browser_sources = ["browser-extension/background.js"]
        if sid in ("browser.popup-status-feedback", "browser.feedback-overview"):
            browser_sources += ["browser-extension/popup.html", "browser-extension/popup.js", "browser-extension/popup.css"]
        if sid in ("browser.feedback-overview", "browser.delivery-acknowledgement"):
            browser_sources += ["src/app/App.tsx"]
            s.setdefault("observations", []).append("Cockpit FeedbackPanel acknowledgement is a separate native workbench surface; popup acknowledgement evidence must not be attributed to FeedbackPanel.")
        if sid in ("browser.annotation-overlay-tools", "browser.capture"):
            browser_sources += ["browser-extension/content.js"]
        s["authority"] = list(dict.fromkeys(s.get("authority", []) + browser_sources))
        s.setdefault("friction", []).append("Checkpoint handoff observations are historical; current source confirms implementation shape, not fresh browser runtime behavior.")
    if s.get("result") not in ("PASS", "FAIL", "INCONCLUSIVE"):
        s["result"] = "INCONCLUSIVE"
    if s.get("environment", {}).get("commit") != BASELINE:
        s.setdefault("friction", []).append("Evidence is not fresh for atlas baseline; runtime acceptance gap remains explicit.")
    s["artifacts"] = [artifact(a, d) for a in s.get("artifacts", [])]
    return s

records = []
for d in (ROOT / "workbench-terminal", ROOT / "browser-extension", ROOT / "setup-context-review"):
    for f in sorted(d.glob("*.json")):
        if f.name in ("index.json", "coverage.json"): continue
        obj = json.loads(f.read_text())
        obj = normalize(obj, d)
        f.write_text(json.dumps(obj, indent=2) + "\n")

for f in (ROOT / "browser-extension/index.json", ROOT / "setup-context-review/index.json"):
    obj = json.loads(f.read_text())
    obj["scenarios"] = [normalize(s, f.parent) for s in obj.get("scenarios", [])]
    f.write_text(json.dumps(obj, indent=2) + "\n")
    records.extend((s["id"], f.relative_to(ROOT).as_posix()) for s in obj["scenarios"])
f = ROOT / "workbench-terminal/index.json"
obj = json.loads(f.read_text())
for x in obj["scenarios"]:
    x["result"] = "INCONCLUSIVE" if x.get("result") not in ("PASS", "FAIL", "INCONCLUSIVE") else x["result"]
    x["primaryCategory"], x["secondaryCategories"] = categories(x["id"])
obj["artifacts"] = [artifact(a, f.parent) for a in obj.get("artifacts", [])]
f.write_text(json.dumps(obj, indent=2) + "\n")
records.extend((x["id"], f.relative_to(ROOT).as_posix()) for x in obj["scenarios"])

cf = ROOT / "setup-context-review/coverage.json"
cov = json.loads(cf.read_text())
cov["currentAcceptanceEvidence"] = [
    {"source": "planning/next-level/execution/run-20260904T214621Z/WORKFLOW_ACCEPTANCE.md", "date": "2026-09-05", "commit": "a1fb90c", "covers": ["Context workflow increment", "setup creation", "typed teardown", "dirty-worktree refusal", "durable orphan recovery", "snapshot import"]},
    {"source": "planning/next-level/execution/run-20260904T214621Z/FEATURE_ACCEPTANCE.md", "date": "2026-09-05", "commit": "a1fb90c", "covers": ["SRC-01/02/03 source hydration", "source refresh result", "snapshot import", "Review and paste acceptance"]},
]
cov["acceptanceChronology"] = "2026-09-05 FEATURE_ACCEPTANCE and WORKFLOW_ACCEPTANCE describe the completed increment; 2026-09-06 daily-use evidence is later runtime evidence but predates this atlas baseline."
for row in cov["coverage"]:
    if row["scenario"] in ("setup.repository-artifact-to-workspace", "setup.recovery-and-teardown", "context.sources-import-and-snapshot"):
        row["status"] = "implemented-static-runtime-gap"
        row["static"] = "current-acceptance-and-source"
        row["runtime"] = "none"
cov["evidenceClasses"]["openGaps"] = [g for g in cov["evidenceClasses"]["openGaps"] if "setup" not in g.lower() or "runtime" in g.lower()]
cov["evidenceClasses"]["openGaps"].extend(["No current-baseline runtime setup receipt or screenshot.", "No source import/refresh/remove or local snapshot runtime proof; implementation is established by current source and acceptance records."])
for a in cov["evidenceClasses"].get("durableScreenshotEvidence", []):
    norm = artifact(a, cf.parent)
    norm["path"] = a.get("path")
    cov["evidenceClasses"]["durableScreenshotEvidence"][cov["evidenceClasses"]["durableScreenshotEvidence"].index(a)] = norm
cf.write_text(json.dumps(cov, indent=2) + "\n")
manifest={"schemaVersion":1,"baselineCommit":BASELINE,"createdOn":"2026-09-12","status":"audited-inconclusive","safety":{"protectedHerdrSessions":["default"],"runtimePolicy":"Use uniquely named disposable sessions and isolated browser profiles only.","browserPolicy":"Do not navigate, capture, or modify unrelated user tabs or profiles."},"evidenceSummary":{"static":"Current-baseline source and dated acceptance records indexed; historical runtime captures retained with provenance.","runtime":"No fresh runtime capture was performed for this atlas repair; stale captures are INCONCLUSIVE, never current proof.","designReferences":"Copied planning/ui-polish PNGs are mock design references with copy SHA-256 hashes."},"gapSummary":["Fresh runtime remains required for lifecycle, rendering, focus/input, provider, extension, and recovery claims.","Historical browser checkpoint be68fc8 and daily-use 7084603 evidence do not establish baseline 1d09b5c behavior."],"scenarios":[{"id":sid,"file":path} for sid,path in sorted(records)]}
(ROOT/"manifest.json").write_text(json.dumps(manifest, indent=2)+"\n")

# Validation mode: parse every JSON and enforce scenario contract.
if __name__ == "__main__":
    allowed={"PASS","FAIL","INCONCLUSIVE"}; ids=[]
    for f in ROOT.rglob("*.json"):
        json.loads(f.read_text())
    m=json.loads((ROOT/"manifest.json").read_text())
    for e in m["scenarios"]:
        p=ROOT/e["file"]; assert p.exists(), e
        o=json.loads(p.read_text()); found=o.get("scenarios", o if "id" in o else {})
        if isinstance(found, list): assert any(s["id"]==e["id"] for s in found)
        else: assert found["id"]==e["id"]
    for f in ROOT.rglob("*.json"):
        if f.name in ("scenario-template.json", "coverage-taxonomy.json", "evidence-rules.json", "manifest.json"): continue
        o=json.loads(f.read_text()); ss=o.get("scenarios", [o] if "id" in o else [])
        for s in ss:
            if "id" not in s or "result" not in s: continue
            assert s["result"] in allowed and s["primaryCategory"] and isinstance(s["secondaryCategories"],list)
    print(f"atlas valid: {len(m['scenarios'])} scenarios")
