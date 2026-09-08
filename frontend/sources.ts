// Workout-library provider registry — the plugin surface for the Workouts
// screen and the Libraries screen. Each provider declares its identity, its
// (optional) config fields, whether it has a connection test, and its tab body.
//
// To add a provider: implement its backend fetch commands + tab component, then
// append one descriptor here. No settings-schema change is needed — config is a
// stringly-typed values bag keyed by the `fields` below (see Rust SourceConfig).
// trainer-coach reuses this same descriptor shape for its own load sources.

import type { ComponentType } from "react";
import PlannerTab from "./screens/PlannerTab";
import WozTab from "./screens/WozTab";

export type FieldKind = "text" | "url" | "password";

export interface SourceField {
  key: string;
  label: string;
  kind: FieldKind;
  placeholder?: string;
}

export interface WorkoutSourceDef {
  id: string;
  label: string;
  /** Tab body in Workouts. `null` = the built-in local library grid. */
  Component: ComponentType | null;
  /** Built-in (local files): always on, never listed/toggled in Libraries. */
  builtin?: boolean;
  /** One-line description shown in the Libraries list. */
  description?: string;
  /** Config fields; empty = enable/disable only. */
  fields: SourceField[];
  /** Whether the provider exposes a connection test (source_test). */
  testable: boolean;
}

export const SOURCES: WorkoutSourceDef[] = [
  {
    id: "local",
    label: "My Library",
    Component: null,
    builtin: true,
    fields: [],
    testable: false,
  },
  {
    id: "woz",
    label: "Zwift",
    Component: WozTab,
    description: "Browse Zwift's built-in workouts (whatsonzwift).",
    fields: [],
    testable: false,
  },
  {
    id: "planner",
    label: "WorkoutPlanner",
    Component: PlannerTab,
    description: "Your workouts from a WorkoutPlanner server.",
    fields: [
      { key: "url", label: "Server URL", kind: "url", placeholder: "https://your-workoutplanner.example.com" },
      { key: "user", label: "Username (Basic Auth)", kind: "text" },
      { key: "pass", label: "Password", kind: "password" },
    ],
    testable: true,
  },
];

/** Providers a user can enable/disable and configure (excludes the built-in). */
export const CONFIGURABLE_SOURCES = SOURCES.filter((s) => !s.builtin);
