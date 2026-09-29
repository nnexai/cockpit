// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { routeWorkbenchKeydown, type WorkbenchKeyRouting } from "./keymap";

afterEach(() => document.body.replaceChildren());

type EventOverrides = Partial<{ key: string; shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean; target: EventTarget | null; isComposing: boolean }>;

function keyEvent(overrides: EventOverrides = {}): KeyboardEvent {
  return {
    key: "z", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: null, isComposing: false,
    preventDefault: vi.fn(), stopPropagation: vi.fn(),
    ...overrides,
  } as unknown as KeyboardEvent;
}

function harness(modalOpen = false) {
  const runCommand = vi.fn();
  const setCommandsOpen = vi.fn();
  const onUnboundPrefixKey = vi.fn();
  let prefixActive = false;
  const setPrefixActive = vi.fn((active: boolean) => { prefixActive = active; });
  const route = (event: KeyboardEvent) => {
    const routing: WorkbenchKeyRouting = { modalOpen, prefixActive, runCommand, setPrefixActive, setCommandsOpen, onUnboundPrefixKey };
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
