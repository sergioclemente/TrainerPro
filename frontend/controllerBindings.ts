import type { ControllerButton, ControllerProfile } from "./ipc";

interface ControllerBinding {
  talk: ControllerButton;
  pauseResume: ControllerButton;
  help: string;
}

const bindings: Record<ControllerProfile, ControllerBinding> = {
  wahoo_virtual_bike: {
    talk: "right_steer",
    pauseResume: "left_steer",
    help: "Left steering: pause/resume · hold right steering: talk",
  },
  zwift_ride: {
    talk: "y",
    pauseResume: "a",
    help: "A: pause/resume · hold Y: talk",
  },
};

export function controllerBinding(profile: ControllerProfile | null): ControllerBinding | null {
  return profile ? bindings[profile] ?? null : null;
}
