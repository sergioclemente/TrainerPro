import type { ComponentType } from "react";
import GarminSignIn from "./GarminSignIn";
import IntervalsSignIn from "./IntervalsSignIn";

export interface SignInProps {
  disabled: boolean;
  onConnected: () => Promise<void>;
}

// Authentication forms are provider-specific. The backend owns capabilities.
export const SIGN_IN_FORMS: Record<string, ComponentType<SignInProps>> = {
  garmin: GarminSignIn,
  intervals_icu: IntervalsSignIn,
};
