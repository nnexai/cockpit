// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { routeWorkbenchKeydown, type WorkbenchKeyRouting } from "./keymap";
import { herdrBindings, herdrPrefixes, herdrCommandShortcut } from "./herdrBindings";

afterEach(() => document.body.replaceChildren());

type EventOverrides = Partial<{ key: string; code: string; repeat: boolean; shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean; target: EventTarget | null; isComposing: boolean }>;

function keyEvent(overrides: EventOverrides = {}): KeyboardEvent {
  return {
    key: "z", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: null, isComposing: false,
    preventDefault: vi.fn(), stopPropagation: vi.fn(),
    ...overrides,
  } as unknown as KeyboardEvent;
}

function harness(modalOpen = false, custom: Partial<WorkbenchKeyRouting> = {}) {
  const runCommand = vi.fn();
  const setCommandsOpen = vi.fn();
  const onUnboundPrefixKey = vi.fn();
  let prefixActive = false;
  let prefixOrigin: "cockpit" | "herdr" = "cockpit";
  const setPrefixActive = vi.fn((active: boolean) => { prefixActive = active; });
  const route = (event: KeyboardEvent) => {
    const routing: WorkbenchKeyRouting = { modalOpen, prefixActive, prefixOrigin, herdrPrefixes: herdrPrefixes(custom.herdrBindings ? ["ctrl+b"] : []), onPrefixArm: origin => { prefixOrigin = origin; }, runCommand, setPrefixActive, setCommandsOpen, onUnboundPrefixKey, ...custom };
    routeWorkbenchKeydown(event, routing);
  };
  return { route, runCommand, setCommandsOpen, setPrefixActive, onUnboundPrefixKey, armed: () => prefixActive };
}

function mount(html: string, selector: string): HTMLElement {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  return host.querySelector<HTMLElement>(selector)!;
}

const ctrlB = () => keyEvent({ key: "b", ctrlKey: true });

describe("prefix routing", () => {
  it("arms on Ctrl+B, runs the next bound key, and leaves ordinary typing alone", () => {
    const { route, runCommand, armed } = harness();
    const ordinary = keyEvent({ key: "z" });
    route(ordinary);
    expect(ordinary.preventDefault).not.toHaveBeenCalled();

    const prefix = ctrlB();
    route(prefix);
    expect(prefix.preventDefault).toHaveBeenCalledOnce();
    expect(prefix.stopPropagation).toHaveBeenCalledOnce();
    expect(armed()).toBe(true);

    const command = keyEvent({ key: "z" });
    route(command);
    expect(command.preventDefault).toHaveBeenCalledOnce();
    expect(command.stopPropagation).toHaveBeenCalledOnce();
    expect(runCommand).toHaveBeenCalledExactlyOnceWith("zoom-pane");
    expect(armed()).toBe(false);
  });

  it("keeps the prefix armed across modifier-only keydowns until the shifted command arrives", () => {
    const { route, runCommand, armed } = harness();
    route(ctrlB());
    for (const modifier of [keyEvent({ key: "Shift", shiftKey: true }), keyEvent({ key: "Control", ctrlKey: true }), keyEvent({ key: "Alt", altKey: true }), keyEvent({ key: "Meta", metaKey: true })]) route(modifier);
    expect(armed()).toBe(true);
    expect(runCommand).not.toHaveBeenCalled();
    route(keyEvent({ key: "N", shiftKey: true }));
    expect(runCommand).toHaveBeenCalledExactlyOnceWith("new-space");
  });

  it("swallows an unbound key, disarms and says so; a Herdr-only key is unbound", () => {
    const { route, runCommand, onUnboundPrefixKey, armed } = harness();
    for (const [key, shiftKey, message] of [["q", false, "Ctrl+B q is not bound in Cockpit"], ["o", false, "Ctrl+B o is not bound in Cockpit"], ["O", true, "Ctrl+B Shift+O is not bound in Cockpit"], ["[", false, "Ctrl+B [ is not bound in Cockpit"]] as const) {
      route(ctrlB());
      const event = keyEvent({ key, shiftKey });
      route(event);
      expect(event.preventDefault).toHaveBeenCalledOnce();
      expect(event.stopPropagation).not.toHaveBeenCalled();
      expect(onUnboundPrefixKey).toHaveBeenLastCalledWith(message);
      expect(armed()).toBe(false);
    }
    expect(runCommand).not.toHaveBeenCalled();
  });

  it("cancels an armed prefix with Escape and consumes only that key", () => {
    const { route, runCommand, armed } = harness();
    route(ctrlB());
    const escape = keyEvent({ key: "Escape" });
    route(escape);
    expect(escape.preventDefault).toHaveBeenCalledOnce();
    expect(escape.stopPropagation).toHaveBeenCalledOnce();
    expect(armed()).toBe(false);
    expect(runCommand).not.toHaveBeenCalled();
  });

  it("lets other Ctrl, Alt and Meta chords through while armed", () => {
    const { route, runCommand, armed } = harness();
    route(ctrlB());
    const chord = keyEvent({ key: "z", ctrlKey: true });
    route(chord);
    expect(chord.preventDefault).not.toHaveBeenCalled();
    expect(chord.stopPropagation).not.toHaveBeenCalled();
    expect(runCommand).not.toHaveBeenCalled();
    expect(armed()).toBe(false);
  });

  it("ignores the prefix while a modal owns the keyboard or during composition", () => {
    const modal = harness(true);
    const event = ctrlB();
    modal.route(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(modal.setPrefixActive).not.toHaveBeenCalled();
    const composing = harness();
    composing.route(keyEvent({ key: "b", ctrlKey: true, isComposing: true }));
    expect(composing.setPrefixActive).not.toHaveBeenCalled();
  });
});

describe("quick switcher chord", () => {
  it.each([
    ['<div class="terminal-host"><textarea></textarea></div>', "textarea"],
    ['<div class="browser-surface" tabindex="0"></div>', ".browser-surface"],
    ['<input aria-label="Search" />', "input"],
  ])("claims Ctrl+K before content input in %s", (html, selector) => {
    const target = mount(html, selector);
    const onQuickSwitch = vi.fn();
    const { route, armed } = harness(false, { onQuickSwitch });
    route(ctrlB());
    const event = keyEvent({ key: "k", ctrlKey: true, target });
    route(event);
    expect(onQuickSwitch).toHaveBeenCalledOnce();
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(armed()).toBe(false);
  });

  it("leaves modal and composing input alone, including DOM-only dialogs", () => {
    const onQuickSwitch = vi.fn();
    const target = mount('<section role="dialog" aria-modal="true"><input /></section>', "input");
    for (const [routing, overrides] of [
      [harness(false, { onQuickSwitch }), { target }],
      [harness(true, { onQuickSwitch }), {}],
      [harness(false, { onQuickSwitch }), { isComposing: true }],
      [harness(false, { onQuickSwitch, serverModalOpen: true }), {}],
    ] as const) {
      const event = keyEvent({ key: "k", ctrlKey: true, ...overrides });
      routing.route(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(onQuickSwitch).not.toHaveBeenCalled();
  });

  it("claims only the exact platform chord, not other modifier variants", () => {
    const onQuickSwitch = vi.fn();
    const { route } = harness(false, { onQuickSwitch });
    for (const overrides of [{ shiftKey: true }, { altKey: true }, { metaKey: true }, { ctrlKey: false, metaKey: true }]) {
      const event = keyEvent({ key: "k", ctrlKey: true, ...overrides });
      route(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(onQuickSwitch).not.toHaveBeenCalled();
    const platform = vi.spyOn(window.navigator, "platform", "get").mockReturnValue("MacIntel");
    try {
      const control = keyEvent({ key: "k", ctrlKey: true });
      route(control);
      expect(control.preventDefault).not.toHaveBeenCalled();
      const command = keyEvent({ key: "k", metaKey: true });
      route(command);
      expect(command.preventDefault).toHaveBeenCalledOnce();
      expect(onQuickSwitch).toHaveBeenCalledOnce();
    } finally { platform.mockRestore(); }
  });

  it("preserves an advertised direct Herdr binding over the local chord", () => {
    const command = { command_id: "custom:k", binding_labels: ["ctrl+k"], action: "shell" as const, description: "Custom K" };
    const onQuickSwitch = vi.fn();
    const runHerdrCommand = vi.fn();
    const { route } = harness(false, { onQuickSwitch, runHerdrCommand, herdrBindings: herdrBindings([command]) });
    const event = keyEvent({ key: "k", ctrlKey: true });
    route(event);
    expect(runHerdrCommand).toHaveBeenCalledExactlyOnceWith(command);
    expect(onQuickSwitch).not.toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalledOnce();
  });

  it("preserves a configured Herdr prefix and its double-prefix passthrough", () => {
    const onQuickSwitch = vi.fn();
    const command = { command_id: "prefix:k", binding_labels: ["prefix+a"], action: "plugin_action" as const, description: "Prefix action" };
    const runHerdrCommand = vi.fn();
    const { route, armed } = harness(false, { onQuickSwitch, runHerdrCommand, herdrBindings: herdrBindings([command]), herdrPrefixes: herdrPrefixes(["ctrl+k"]) });
    const prefix = keyEvent({ key: "k", ctrlKey: true });
    route(prefix);
    expect(armed()).toBe(true);
    expect(prefix.preventDefault).toHaveBeenCalledOnce();
    const literal = keyEvent({ key: "k", ctrlKey: true });
    route(literal);
    expect(armed()).toBe(false);
    expect(literal.preventDefault).not.toHaveBeenCalled();
    route(keyEvent({ key: "k", ctrlKey: true }));
    route(keyEvent({ key: "a" }));
    expect(runHerdrCommand).toHaveBeenCalledExactlyOnceWith(command);
    expect(onQuickSwitch).not.toHaveBeenCalled();
  });
});

describe("direct quick chords", () => {
  it.each([
    [{ key: "`", code: "Backquote", ctrlKey: true }, "quick-terminal"],
    [{ key: "A", code: "KeyA", ctrlKey: true, shiftKey: true }, "ask-agent"],
    [{ key: "A", code: "KeyA", metaKey: true, shiftKey: true }, "ask-agent"],
    [{ key: "L", code: "KeyL", ctrlKey: true, shiftKey: true }, "liaison"],
    [{ key: "L", code: "KeyL", metaKey: true, shiftKey: true }, "liaison"],
  ] as const)("routes %s from a terminal to %s", (keys, command) => {
    const target = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    const { route, runCommand, armed } = harness();
    const event = keyEvent({ ...keys, target });
    route(event);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(runCommand).toHaveBeenCalledExactlyOnceWith(command);
    expect(armed()).toBe(false);
  });

  it("routes the liaison chord ahead of an armed prefix and ignores repeats and modals", () => {
    const { route, runCommand, armed } = harness();
    route(ctrlB());
    route(keyEvent({ key: "L", ctrlKey: true, shiftKey: true }));
    expect(runCommand).toHaveBeenCalledExactlyOnceWith("liaison");
    expect(armed()).toBe(false);
    route(keyEvent({ key: "L", ctrlKey: true, shiftKey: true, repeat: true }));
    expect(runCommand).toHaveBeenCalledOnce();
    const modal = harness(true);
    const event = keyEvent({ key: "L", ctrlKey: true, shiftKey: true });
    modal.route(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(modal.runCommand).not.toHaveBeenCalled();
  });

  it("consumes repeats without running the command again and leaves modal input alone", () => {
    const target = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    const { route, runCommand } = harness();
    const repeat = keyEvent({ key: "`", code: "Backquote", ctrlKey: true, repeat: true, target });
    route(repeat);
    expect(repeat.preventDefault).toHaveBeenCalledOnce();
    expect(runCommand).not.toHaveBeenCalled();
    const modal = harness(true);
    const event = keyEvent({ key: "A", ctrlKey: true, shiftKey: true, target });
    modal.route(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(modal.runCommand).not.toHaveBeenCalled();
  });
});

describe("terminal and browser focus", () => {
  it("never consumes a plain Tab or Shift+Tab in a terminal, armed or not", () => {
    const { route, runCommand, setPrefixActive } = harness();
    const textarea = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    for (const shiftKey of [false, true]) {
      const tab = keyEvent({ key: "Tab", shiftKey, target: textarea });
      route(tab);
      expect(tab.preventDefault).not.toHaveBeenCalled();
      expect(tab.stopPropagation).not.toHaveBeenCalled();
    }
    expect(runCommand).not.toHaveBeenCalled();
    expect(setPrefixActive).not.toHaveBeenCalled();
  });

  it("reads Tab and Shift+Tab as pane cycling only right after the prefix", () => {
    const { route, runCommand } = harness();
    const textarea = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    route(keyEvent({ key: "b", ctrlKey: true, target: textarea }));
    route(keyEvent({ key: "Tab", target: textarea }));
    route(keyEvent({ key: "b", ctrlKey: true, target: textarea }));
    route(keyEvent({ key: "Shift", shiftKey: true, target: textarea }));
    route(keyEvent({ key: "Tab", shiftKey: true, target: textarea }));
    expect(runCommand.mock.calls).toEqual([["next-pane"], ["previous-pane"]]);
  });

  it("sends a literal Ctrl+B through on Ctrl+B Ctrl+B", () => {
    const { route, runCommand, armed } = harness();
    const textarea = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    route(keyEvent({ key: "b", ctrlKey: true, target: textarea }));
    const literal = keyEvent({ key: "b", ctrlKey: true, target: textarea });
    route(literal);
    expect(literal.preventDefault).not.toHaveBeenCalled();
    expect(literal.stopPropagation).not.toHaveBeenCalled();
    expect(armed()).toBe(false);
    expect(runCommand).not.toHaveBeenCalled();
  });

  it("does not open Commands for a bare ? typed into a terminal", () => {
    const { route, setCommandsOpen } = harness();
    const textarea = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    const question = keyEvent({ key: "?", target: textarea });
    route(question);
    expect(setCommandsOpen).not.toHaveBeenCalled();
    expect(question.preventDefault).not.toHaveBeenCalled();
    const button = mount("<button>go</button>", "button");
    route(keyEvent({ key: "?", target: button }));
    expect(setCommandsOpen).toHaveBeenCalledWith(true);
  });

  it("arms from the browser surface but not from browser chrome text fields", () => {
    const surface = mount('<section class="browser-pane"><input class="url" /><div class="browser-surface" tabindex="0"></div></section>', ".browser-surface");
    const input = document.querySelector<HTMLElement>(".url")!;
    const fromSurface = harness();
    const arm = keyEvent({ key: "b", ctrlKey: true, target: surface });
    fromSurface.route(arm);
    expect(arm.preventDefault).toHaveBeenCalledOnce();
    fromSurface.route(keyEvent({ key: "2", target: surface }));
    expect(fromSurface.runCommand).toHaveBeenCalledExactlyOnceWith("select-tab-2");

    const fromInput = harness();
    const ignored = keyEvent({ key: "b", ctrlKey: true, target: input });
    fromInput.route(ignored);
    expect(ignored.preventDefault).not.toHaveBeenCalled();
    expect(fromInput.armed()).toBe(false);
  });
});

describe("dialogs", () => {
  it("leaves prefix commands inside an open comment dialog", () => {
    const button = mount("<dialog open><button>Save comment</button></dialog>", "button");
    button.focus();
    const { route, runCommand, setPrefixActive, setCommandsOpen } = harness();
    const target = { target: button };
    route(keyEvent({ key: "b", ctrlKey: true, ...target }));
    for (const key of ["c", "v", "x", "?"]) route(keyEvent({ key, ...target }));
    expect(runCommand).not.toHaveBeenCalled();
    expect(setPrefixActive).not.toHaveBeenCalled();
    expect(setCommandsOpen).not.toHaveBeenCalled();
  });

  it("treats a Library-internal aria-modal dialog as modal, but not the narrow sidebar drawer", () => {
    const inDialog = mount('<section role="dialog" aria-modal="true"><button>Add</button></section>', "button");
    const dialogRoute = harness();
    dialogRoute.route(keyEvent({ key: "b", ctrlKey: true, target: inDialog }));
    expect(dialogRoute.setPrefixActive).not.toHaveBeenCalled();

    const inDrawer = mount('<aside id="cockpit-sidebar" role="dialog" aria-modal="true"><button>Row</button></aside>', "#cockpit-sidebar button");
    const drawerRoute = harness();
    drawerRoute.route(keyEvent({ key: "b", ctrlKey: true, target: inDrawer }));
    expect(drawerRoute.armed()).toBe(true);
  });
});

describe("advertised Herdr command routing", () => {
  const command = { command_id: "opaque:17", binding_labels: ["prefix+alt+a", "prefix+f", "alt+z"], action: "plugin_action" as const, description: "Configured action" };
  it("uses every advertised prefix alias without moving Cockpit commands onto that prefix", () => {
    const runHerdrCommand = vi.fn();
    const prefixes = herdrPrefixes(["ctrl+a", "alt+space"]);
    const routed = harness(false, { herdrBindings: herdrBindings([command]), herdrPrefixes: prefixes, runHerdrCommand });
    routed.route(ctrlB());
    routed.route(keyEvent({ key: "f" }));
    expect(routed.runCommand).toHaveBeenCalledExactlyOnceWith("open-file-picker");
    expect(runHerdrCommand).not.toHaveBeenCalled();
    routed.route(keyEvent({ key: "a", ctrlKey: true }));
    routed.route(keyEvent({ key: "å", code: "KeyA", altKey: true }));
    routed.route(keyEvent({ key: " ", altKey: true }));
    routed.route(keyEvent({ key: "f" }));
    expect(runHerdrCommand.mock.calls).toEqual([[command], [command]]);
    routed.route(keyEvent({ key: "a", ctrlKey: true }));
    const literal = keyEvent({ key: "a", ctrlKey: true });
    routed.route(literal);
    expect(literal.preventDefault).not.toHaveBeenCalled();
    expect(routed.armed()).toBe(false);
    expect(herdrCommandShortcut(command, prefixes)).toContain("Ctrl+A Alt+A");
    expect(herdrCommandShortcut(command, prefixes)).toContain("Alt+Space F");
  });
  it("uses physical Alt letters and overrides Cockpit prefix actions", () => {
    const runHerdrCommand = vi.fn();
    const routed = harness(false, { herdrBindings: herdrBindings([command]), runHerdrCommand });
    routed.route(ctrlB());
    const option = keyEvent({ key: "å", code: "KeyA", altKey: true });
    routed.route(option);
    expect(option.preventDefault).toHaveBeenCalledOnce();
    expect(routed.armed()).toBe(false);
    routed.route(ctrlB());
    routed.route(keyEvent({ key: "f" }));
    expect(runHerdrCommand.mock.calls).toEqual([[command], [command]]);
    expect(routed.runCommand).not.toHaveBeenCalled();
  });
  it("uses active-layout Alt letters instead of claiming their physical QWERTY positions", () => {
    const inbox = { command_id: "opaque:inbox", binding_labels: ["prefix+alt+i"], action: "shell" as const, description: "Inbox" };
    const runHerdrCommand = vi.fn();
    const routed = harness(false, { herdrBindings: herdrBindings([inbox]), runHerdrCommand });
    routed.route(ctrlB());
    const otherLetter = keyEvent({ key: "l", code: "KeyI", altKey: true });
    routed.route(otherLetter);
    expect(otherLetter.preventDefault).not.toHaveBeenCalled();
    expect(runHerdrCommand).not.toHaveBeenCalled();
    routed.route(ctrlB());
    const colemak = keyEvent({ key: "i", code: "KeyL", altKey: true });
    routed.route(colemak);
    expect(colemak.preventDefault).toHaveBeenCalledOnce();
    expect(runHerdrCommand).toHaveBeenCalledExactlyOnceWith(inbox);
    expect(routed.runCommand).not.toHaveBeenCalled();
    expect(routed.armed()).toBe(false);
  });
  it("retains the physical Alt letter fallback for dead keys", () => {
    const inbox = { command_id: "opaque:inbox", binding_labels: ["prefix+alt+i"], action: "shell" as const, description: "Inbox" };
    const runHerdrCommand = vi.fn();
    const routed = harness(false, { herdrBindings: herdrBindings([inbox]), runHerdrCommand });
    routed.route(ctrlB());
    const deadKey = keyEvent({ key: "Dead", code: "KeyI", altKey: true });
    routed.route(deadKey);
    expect(deadKey.preventDefault).toHaveBeenCalledOnce();
    expect(runHerdrCommand).toHaveBeenCalledExactlyOnceWith(inbox);
  });
  it("claims direct chords only on prefix-safe targets and never invokes repeats", () => {
    const runHerdrCommand = vi.fn();
    const routed = harness(false, { herdrBindings: herdrBindings([command]), runHerdrCommand });
    const terminal = mount('<div class="terminal-host"><textarea></textarea></div>', "textarea");
    const editor = mount("<input />", "input");
    routed.route(keyEvent({ key: "z", code: "KeyZ", altKey: true, target: editor }));
    routed.route(keyEvent({ key: "z", code: "KeyZ", altKey: true, target: terminal, isComposing: true }));
    routed.route(keyEvent({ key: "z", code: "KeyZ", altKey: true, target: terminal, repeat: true }));
    expect(runHerdrCommand).not.toHaveBeenCalled();
    routed.route(keyEvent({ key: "z", code: "KeyZ", altKey: true, target: terminal }));
    expect(runHerdrCommand).toHaveBeenCalledExactlyOnceWith(command);
  });
  it("does not claim unknown actions, nonmatching modifiers or keys owned by server popups", () => {
    const runHerdrCommand = vi.fn();
    const routed = harness(false, { herdrBindings: herdrBindings([{ ...command, action: "unknown" }]), runHerdrCommand });
    const ignored = keyEvent({ key: "z", altKey: true });
    routed.route(ignored);
    expect(ignored.preventDefault).not.toHaveBeenCalled();
    const popup = harness(false, { serverModalOpen: true, prefixActive: true, herdrBindings: herdrBindings([command]), runHerdrCommand });
    for (const event of [keyEvent({ key: "Escape" }), keyEvent({ key: "Tab" }), ctrlB(), keyEvent({ key: "z", altKey: true })]) {
      popup.route(event);
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(runHerdrCommand).not.toHaveBeenCalled();
    const pending = harness(false, { popupPending: true });
    const text = keyEvent({ key: "x" });
    pending.route(text);
    expect(text.preventDefault).toHaveBeenCalledOnce();
    expect(text.stopPropagation).toHaveBeenCalledOnce();
  });
});
