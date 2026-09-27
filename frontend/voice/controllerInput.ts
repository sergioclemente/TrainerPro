import type { ControllerButton, ControllerInputEvent } from "../ipc";
import { controllerBinding } from "../controllerBindings";

interface ControllerActions {
  begin: () => void;
  finish: () => void;
  cancel: () => void;
  pauseResume: () => void;
  visible: () => boolean;
}

/** Owns button continuity; cancellation never becomes a submit/release. */
export function createControllerInputHandler(actions: ControllerActions) {
  let generation = -1;
  const held = new Set<ControllerButton>();
  const blocked = new Set<ControllerButton>();
  return (input: ControllerInputEvent) => {
    if (input.generation < generation) return;
    if (input.generation !== generation || input.event.kind === "cancel") {
      for (const button of held) blocked.add(button);
      held.clear();
      generation = input.generation;
      actions.cancel();
    }
    if (input.event.kind === "cancel") return;
    const binding = controllerBinding(input.profile);
    if (!binding) return;
    const { button, pressed } = input.event;
    if (!pressed) {
      blocked.delete(button);
      if (!held.delete(button)) return;
    } else {
      if (blocked.has(button) || held.has(button)) return;
      held.add(button);
    }
    if (!actions.visible()) { actions.cancel(); return; }
    if (button === binding.talk) {
      if (pressed) actions.begin(); else actions.finish();
    } else if (button === binding.pauseResume && pressed) actions.pauseResume();
  };
}

export function isPushToTalkKey(event: Pick<KeyboardEvent, "code" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">): boolean {
  return event.code === "Space" && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

export function isInteractiveTarget(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest(
    "input, textarea, select, button, a[href], [contenteditable]:not([contenteditable='false']), [role='button'], [role='textbox'], [role='slider']",
  );
}
