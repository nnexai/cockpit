import type { ReactElement } from "react";
import { LibraryMenu, type LibraryMenuEntry } from "../library/LibraryTree";
import { moveIntent } from "./stepInteractions";
import type { StepMenuState } from "./stepViewTypes";
import { requiresPreview, type StepMutation } from "./useStepMutation";
import type { StepText } from "./useStepText";

export function StepMenu({ menu, setMenu, mutation, text }: {
  menu: StepMenuState | null; setMenu: (menu: StepMenuState | null) => void;
  mutation: StepMutation; text: StepText;
}): ReactElement | null {
  const { blockedReason, guard, submission, preview, send } = mutation;
  const { openText } = text;
  const menuEntries: LibraryMenuEntry[] = menu ? (() => {
    const selected = menu.task.steps.find((step) => step.step_id === menu.stepId);
    const disabled = blockedReason !== null;
    function move(direction: "up" | "down" | "indent" | "outdent") {
      const intent = moveIntent(menu!.task, menu!.stepId, direction);
      if (!intent || !guard()) return;
      const submitted = submission(intent, menu!.task.task_revision);
      if (requiresPreview(intent, menu!.task)) preview(submitted, menu!.task); else void send(submitted, menu!.task);
    }
    return [
      { label: "Edit title…", disabled, onSelect: () => openText("rename", menu.stepId, menu.task) },
      { label: "Add sub-step", disabled: disabled || !selected || selected.depth >= 4 || menu.task.steps.filter((step) => step.step_id !== null).length >= 64, onSelect: () => openText("add", menu.stepId, menu.task) },
      "separator",
      ...(["up", "down", "indent", "outdent"] as const).map((direction) => ({
        label: { up: "Move up", down: "Move down", indent: "Indent under previous step", outdent: "Outdent" }[direction],
        disabled: disabled || moveIntent(menu.task, menu.stepId, direction) === null, onSelect: () => move(direction),
      })),
      "separator", { label: "Remove…", destructive: true, disabled, onSelect: () => {
        if (guard()) preview(submission({ kind: "remove", stepId: menu.stepId }, menu.task.task_revision), menu.task);
      } },
    ];
  })() : [];
  return menu ? <div className="supervisor-step-menu-scope" onKeyDownCapture={(event) => {
      if ((event.repeat || event.nativeEvent.isComposing) && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault(); event.stopPropagation();
      }
    }} onKeyDown={(event) => { if (event.defaultPrevented) event.stopPropagation(); }}>
      <LibraryMenu x={menu.x} y={menu.y} label="Step actions" entries={menuEntries} onDismiss={() => {
        setMenu(null);
        if (menu.opener.isConnected) menu.opener.focus({ preventScroll: true });
      }} />
    </div> : null;
}
