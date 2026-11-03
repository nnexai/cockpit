/* Shared renderer for the Inbox polish mocks. MOCK FIXTURE ONLY: no real data, no network.
 * Markup reuses the product's class names (src/app/inbox/Inbox.tsx) so CSS ports back 1:1.
 * <html data-variant="a|b"> selects the few markup differences (see the spec, "Markup deltas").
 * Interactions mirrored from the product: row select, mobile Back, question select/deselect,
 * choice fills the draft and enters Answer mode, per-selection drafts, Send disabled until text.
 * Interactions NOT mirrored: sending, evidence disclosure content, owner/context popovers. */
(() => {
const B = document.documentElement.dataset.variant === "b";
// Icon paths copied from src/app/UiIcon.tsx.
const P = {
  plus: "M12 5v14M5 12h14", right: "m9 5 7 7-7 7", back: "M19 12H5m7 7-7-7 7-7", check: "m4 12 5 5L20 6",
  "check-circle": "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M8 12.3l2.7 2.7L16 9.5",
  eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Zm13 0a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
  help: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M9.4 9.4a2.7 2.7 0 1 1 3.8 2.5c-.8.4-1.2 1-1.2 1.8 M12 17h.01",
  work: "M12 3l9 5-9 5-9-5z M3 13l9 5 9-5", user: "M11 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM3 21a8 8 0 0 1 16 0",
  spark: "M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z M18.5 16l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z",
  send: "M21 3L10 14 M21 3l-7 18-4-7-7-4z", reply: "M9 7L4 12l5 5 M4 12h10a6 6 0 0 1 6 6v1",
  alert: "M12 4l9 16H3z M12 10v4 M12 17h.01", terminal: "m4 6 6 6-6 6m9 0h7",
  grid: "M3 3h7v7H3Zm11 0h7v7h-7ZM3 14h7v7H3Zm11 0h7v7h-7Z",
  comment: "M21 15a3 3 0 0 1-3 3H8l-5 3V6a3 3 0 0 1 3-3h12a3 3 0 0 1 3 3ZM7 8h10M7 12h7",
};
const ic = n => `<svg class="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="${P[n]}"/></svg>`;
const esc = s => String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

// Sample copy. Shapes follow InboxThread / InboxQuestion / UnmanagedAgent in src/protocol/generated/v1.ts.
const T = {
  q: { id: "q", tone: "decision", icon: "help", label: "Needs your decision", project: "Polish sample", age: "4m",
    title: "Choose the scope for the sample update", preview: "Which scope should the sample update use?",
    owner: "Liaison", qtext: "Which scope should the sample update use?",
    choices: ["Sample only", "Sample and fixtures", "Whole repository, including every generated file that references the sample"], rec: "Sample only" },
  r: { id: "r", tone: "review", icon: "eye", label: "Result ready", project: "Supervisor", age: "18m",
    title: "Index repair finished", preview: "Rebuilt the index. No project files changed.", reply: "Rebuilt the index. No project files changed.", owner: "Liaison" },
  w: { id: "w", tone: "work", icon: "work", label: "Working", project: "Polish sample", age: "1h", title: "Update the release checklist", preview: "Waiting for the build to finish" },
  h: { id: "h", tone: "done", icon: "check-circle", label: "Done", project: "Polish sample", age: "2d", title: "Rename the sample Space", preview: "" },
};
const AG = { id: "agent", name: "Alpha", loc: "Polish sample · Tab 1", status: "idle", title: "Repair index", recap: "Index repaired" };
const QUIET_AGENT = { name: "Beta", text: "Polish sample · idle" };
const SCENES = {
  populated: { view: true, needs: ["q", "r"], agent: true, others: ["w", "h"], otherAgents: true },
  empty: { view: true, needs: [], agent: false, others: ["w", "h"], otherAgents: true },
  loading: { view: false },
  firsterror: { view: false, loadError: "Request timed out" },
  error: { view: true, needs: ["q", "r"], agent: true, others: ["w", "h"], otherAgents: true, loadError: "Request timed out", unavailable: true, attn: true, composeError: true },
};
const note = (tone, icon, text, small) => `<p role="status" class="inbox-note"${tone ? ` data-tone="${tone}"` : ""}>${ic(icon)}<span>${text}${small ? `<small>${small}</small>` : ""}</span></p>`;
const time = v => `<time>${v}</time>`;

function rowT(t, selected, quiet) {
  return `<button type="button" class="inbox-row${quiet ? " is-quiet" : ""}" data-tone="${t.tone}" data-act="sel" data-id="${t.id}"${selected ? ' aria-current="true"' : ""}><span class="inbox-kic${quiet ? " is-sm" : ""}" aria-hidden="true">${ic(t.icon)}</span><span class="inbox-row-body"><span class="inbox-meta"><span class="inbox-meta-t"><b class="inbox-state">${t.label}</b> · ${esc(t.project)}</span>${time(t.age)}</span><strong>${esc(t.title)}</strong>${t.preview ? `<span class="inbox-row-next">${esc(t.preview)}</span>` : ""}</span></button>`;
}
function rowAgent(selected) {
  return `<button type="button" class="inbox-row" data-tone="decision" data-act="sel" data-id="agent"${selected ? ' aria-current="true"' : ""}><span class="inbox-kic" aria-hidden="true">${ic("user")}</span><span class="inbox-row-body"><span class="inbox-meta"><span class="inbox-meta-t"><b class="inbox-state">Direct agent</b> · ${esc(AG.loc)}</span></span><strong>${AG.name}</strong><span class="inbox-row-next">${esc(AG.recap)}</span></span></button>`;
}

function list(st, sc, n) {
  const need = sc.view ? sc.needs.length + (sc.agent ? 1 : 0) : 0;
  let h = `<aside class="inbox-list" aria-label="Work needing attention"${sc.view ? "" : ' aria-busy="true"'}><header><h1>Inbox</h1>${need ? `<span class="inbox-count">${need} need you</span>` : ""}<button type="button" class="inbox-new" data-act="new">${ic("plus")} New work</button></header>`;
  if (!sc.view) {
    if (sc.loadError) return h + `<div class="inbox-note" role="status" data-tone="error">${ic("alert")}<span><b>Inbox unavailable.</b> ${esc(sc.loadError)}<small>Retrying automatically.</small></span></div></aside>`;
    return h + `<p class="inbox-loading" role="status">Loading inbox…</p><div class="inbox-skel" aria-hidden="true">${"<div><i></i><span><b></b><b></b></span></div>".repeat(4)}</div></aside>`;
  }
  if (sc.loadError) h += note("error", "alert", "Inbox unavailable. Showing the last loaded items.", `${esc(sc.loadError)}. Retrying automatically.`);
  if (sc.unavailable) h += note("", "alert", "Can’t see live sessions right now. Replies are paused.");
  if (sc.attn) h += note("", "alert", "Agent attention unavailable.");
  if (!need) h += `<p class="inbox-empty">${ic("check-circle")}Nothing needs you right now.</p>`;
  else h += `<section aria-labelledby="g${n}"><h2 class="inbox-group" id="g${n}">Needs you<span>${need}</span></h2>${sc.needs.map(id => rowT(T[id], st.sel === id)).join("")}${sc.agent ? rowAgent(st.sel === "agent") : ""}</section>`;
  const sub = ["underway", "handled"].map((k, i) => { const ids = sc.others.filter(id => (id === "w") === (i === 0)); return ids.length ? `<div><h2 class="inbox-sub">${k === "underway" ? "Underway" : "Handled"}</h2>${ids.map(id => rowT(T[id], st.sel === id, true)).join("")}</div>` : ""; }).join("");
  h += `<details class="inbox-others"${st.others ? " open" : ""}><summary>${ic("right")}Other work · ${sc.others.length}</summary>${sub}</details>`;
  if (sc.otherAgents) h += `<details class="inbox-others"><summary>${ic("right")}Other agents</summary><button type="button" class="inbox-row is-quiet" data-tone="unknown"><span class="inbox-kic is-sm" aria-hidden="true">${ic("user")}</span><span class="inbox-row-body"><strong>${QUIET_AGENT.name}</strong><span class="inbox-row-next">${QUIET_AGENT.text}</span></span></button></details>`;
  return h + "</aside>";
}

const chip = (c, i, st) => {
  const rec = c === T.q.rec, on = st.picked && (st.drafts.q || "") === c;
  const tag = rec ? (B ? `<span class="inbox-rec">${ic("spark")}Recommended</span>` : `${ic("spark")}<span class="inbox-sr">Recommended: </span>`) : "";
  return `<button type="button" class="inbox-chip" data-act="choice" data-i="${i}" aria-pressed="${on}">${B ? `<span class="inbox-chip-t">${esc(c)}</span>${tag}` : `${tag}${esc(c)}`}</button>`;
};
const head = (tone, icon, title, facts) => `<div class="inbox-head" data-tone="${tone}"><span class="inbox-kic is-lg">${ic(icon)}</span><div><h2>${esc(title)}</h2><p class="inbox-facts">${facts}</p></div></div>`;
const details = `<details class="inbox-evidence"><summary>${ic("right")}Details</summary><p class="inbox-ev-p">Next move, your request, evidence, delivery, tasks and IDs render here unchanged (mock placeholder).</p></details>`;

function detail(st, sc) {
  let body, mode, key = st.sel || "new";
  const sel = sc.view ? st.sel : null;
  if (sel === "q") {
    const t = T.q; mode = st.picked ? "answer" : "followup";
    body = `<div class="inbox-content">${head("decision", t.icon, t.title, `<span>${t.project}</span>${time(t.age)}<span class="inbox-state">${t.label}</span><span class="inbox-prov">${ic("spark")}AI summary</span>`)}<section class="inbox-question" data-selected="${st.picked}"><button type="button" class="inbox-q-head" data-act="q" aria-pressed="${st.picked}" title="Ask from ${t.owner}"><span class="inbox-kic is-sm">${ic("help")}</span><span class="inbox-q-who">Ask from ${t.owner}</span>${time("4m")}<span class="inbox-radio">${ic("check")}</span></button><p class="inbox-q-text">${esc(t.qtext)}</p><div class="inbox-choices" role="group" aria-label="Choices from ${t.owner}">${t.choices.map((c, i) => chip(c, i, st)).join("")}</div></section><div class="inbox-actions"><button type="button" class="inbox-btn">${ic("terminal")}<span>Open session</span></button></div>${details}</div>`;
  } else if (sel === "r") {
    const t = T.r; mode = "followup";
    body = `<div class="inbox-content">${head("review", t.icon, t.title, `<span>${t.project}</span>${time(t.age)}<span class="inbox-state">${t.label}</span>`)}<div class="inbox-update"><small>Update from ${t.owner}</small>${esc(t.reply)}</div><div class="inbox-actions"><button type="button" class="inbox-btn"><span>Open session</span></button><button type="button" class="inbox-btn is-primary">${ic("check")}Mark read</button></div>${details}</div>`;
  } else if (sel === "agent") {
    mode = "direct";
    body = `<div class="inbox-content">${head("unknown", "user", AG.name, `<span>${AG.loc}</span><span>Direct agent · ${AG.status}</span>`)}<section aria-label="Session context"><div class="inbox-update inbox-session-context"><small>Session title</small>${AG.title}</div><div class="inbox-update inbox-session-context"><small>Session recap · OMP session</small>${AG.recap}</div></section><div class="inbox-actions"><button type="button" class="inbox-btn">Open session</button></div></div>`;
  } else {
    mode = "new"; key = "new";
    body = `<div class="inbox-content is-new">${head("unknown", "plus", "New work", "<span>Send a request to a liaison.</span>")}</div>`;
  }
  return `<main class="inbox-detail"><button type="button" class="inbox-back" data-act="back" aria-label="Back to inbox">${B ? ic("back") : "‹"} Inbox</button>${body}${compose(st, sc, mode, key)}</main>`;
}

function compose(st, sc, mode, key) {
  const draft = st.drafts[key] || "", pending = sc.composeError && mode !== "new";
  let hd = "";
  if (key === "q") hd += `<div class="inbox-seg" role="group" aria-label="Message type"><button type="button" data-act="followup" aria-pressed="${mode === "followup"}">Follow up</button><button type="button" data-act="answer" aria-pressed="${mode === "answer"}">Answer</button></div>`;
  if (mode === "answer") hd += `<span class="inbox-dest">${ic("reply")}<span>Liaison</span></span>`;
  else if (mode === "followup") hd += `<span class="inbox-dest">${ic("user")}<span>Liaison</span></span>`;
  else if (mode === "new") hd += `<label class="inbox-pick" data-empty="true">${ic("user")}<select aria-label="Owner"><option>Choose owner</option><option>Liaison · available</option></select></label><details class="inbox-ctx"><summary>＋ Context</summary></details>`;
  const ph = { direct: "Message Alpha…", answer: "Type an answer or tap a choice", followup: key === "q" ? "Add an instruction. Doesn’t answer the question" : "Follow up…", new: "Describe the work or ask a question" }[mode];
  const label = { answer: "Answer", new: "Start", direct: "Send direct", followup: "Send" }[mode];
  let notes = "";
  if (!sc.view) notes += note("", "alert", "Sending is available once the inbox has loaded.");
  if (sc.composeError && mode !== "new") notes += `<p role="alert" class="inbox-note" data-tone="error">${ic("alert")}<span>Delivery was not confirmed. The original request is retained; retry it without changing its identity.</span></p>`;
  const send = pending ? `<button type="button" class="inbox-send is-retry">Retry original request</button>` : `<button type="submit" class="inbox-send"${!sc.view || !draft.trim() ? " disabled" : ""}>${ic("send")}${label}</button>`;
  return `<form class="inbox-compose" data-mode="${mode}"><div class="inbox-compose-head">${hd}</div>${notes}<div class="inbox-compose-row"><textarea aria-label="${mode === "new" ? "Your request" : "Message"}" placeholder="${ph}" rows="1"${pending ? " disabled" : ""}>${esc(draft)}</textarea>${send}</div></form>`;
}

let counter = 0;
function render(fig) {
  const st = fig._st, sc = SCENES[st.scene], frame = fig.querySelector(".frame");
  const need = sc.view ? sc.needs.length + (sc.agent ? 1 : 0) : 0;
  frame.innerHTML = `<nav class="view-bar" aria-label="Application views"><button type="button">${ic("grid")} <span class="view-bar-label">Workbench</span></button><button type="button" aria-current="page">${ic("comment")} <span class="view-bar-label">Inbox</span>${need ? `<span class="view-bar-count" aria-hidden="true">${need}</span>` : ""}</button></nav><div class="inbox-root"><section class="inbox" data-detail="${st.detail}" aria-label="Inbox">${list(st, sc, st.n)}${detail(st, sc)}</section></div>`;
  if (st.focus) { fig.querySelector(st.focus)?.focus({ preventScroll: true }); st.focus = ""; }
}
function init(fig) {
  const d = fig.dataset, w = +d.w || 1100, h = +d.h || 620;
  fig.querySelector(".frame").style.cssText = `--w:${w}px;--h:${h}px`;
  fig._st = { scene: d.scene, sel: d.sel || null, detail: d.detail === "1", picked: false, drafts: {}, others: d.others === "1", n: ++counter, focus: "" };
  if (d.picked) { fig._st.picked = true; fig._st.drafts.q = d.picked; }
  fig.addEventListener("click", e => {
    const el = e.target.closest("[data-act]"); if (!el || !fig.contains(el)) return;
    const st = fig._st, act = el.dataset.act, narrow = w <= 700;
    if (act === "sel") { st.sel = el.dataset.id; st.detail = true; st.picked = false; st.focus = narrow ? ".inbox-back" : `[data-act=sel][data-id="${st.sel}"]`; }
    else if (act === "new") { st.sel = null; st.detail = true; st.focus = narrow ? ".inbox-back" : ".inbox-new"; }
    else if (act === "back") { st.detail = false; st.focus = st.sel ? `[data-act=sel][data-id="${st.sel}"]` : ".inbox-new"; }
    else if (act === "q") { st.picked = !st.picked; st.focus = ".inbox-q-head"; }
    else if (act === "choice") { st.picked = true; st.drafts.q = T.q.choices[+el.dataset.i]; st.focus = `.inbox-chip[data-i="${el.dataset.i}"]`; }
    else if (act === "followup") { st.picked = false; st.focus = '[data-act=followup]'; }
    else if (act === "answer") { st.picked = true; st.focus = '[data-act=answer]'; }
    render(fig);
  });
  fig.addEventListener("input", e => {
    if (e.target.tagName !== "TEXTAREA") return;
    const st = fig._st, key = st.sel || "new"; st.drafts[key] = e.target.value;
    const send = fig.querySelector(".inbox-send:not(.is-retry)"); if (send && SCENES[st.scene].view) send.disabled = !e.target.value.trim();
  });
  fig.addEventListener("submit", e => e.preventDefault());
  fig.addEventListener("toggle", e => { if (e.target.classList?.contains("inbox-others") && e.target.querySelector("summary")?.textContent.includes("Other work")) fig._st.others = e.target.open; }, true);
  render(fig);
}
document.querySelectorAll("figure[data-scene]").forEach(init);
})();
