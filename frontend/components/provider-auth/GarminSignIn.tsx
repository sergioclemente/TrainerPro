import { useEffect, useRef, useState } from "react";
import { AppError, ipc } from "../../ipc";

import type { SignInProps } from "./index";

function signInAttemptId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();

  // Older WKWebView versions provide getRandomValues but not randomUUID.
  const UUID_BYTES = 16;
  const VERSION_BYTE = 6;
  const VERSION_PAYLOAD_MASK = 0x0f;
  const VERSION_4_BITS = 0x40;
  const VARIANT_BYTE = 8;
  const VARIANT_PAYLOAD_MASK = 0x3f;
  const RFC_VARIANT_BITS = 0x80;
  const HEX_RADIX = 16;
  const HEX_BYTE_WIDTH = 2;
  const bytes = crypto.getRandomValues(new Uint8Array(UUID_BYTES));
  bytes[VERSION_BYTE] = (bytes[VERSION_BYTE] & VERSION_PAYLOAD_MASK) | VERSION_4_BITS;
  bytes[VARIANT_BYTE] = (bytes[VARIANT_BYTE] & VARIANT_PAYLOAD_MASK) | RFC_VARIANT_BITS;
  const hex = Array.from(bytes, (byte) => byte.toString(HEX_RADIX).padStart(HEX_BYTE_WIDTH, "0")).join("");
  return hex.replace(/^(........)(....)(....)(....)(............)$/, "$1-$2-$3-$4-$5");
}

export default function GarminSignIn({ disabled, onConnected }: SignInProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const mounted = useRef(true);
  const attempt = useRef<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (attempt.current) void ipc.cancelGarminSignIn(attempt.current).catch(() => {});
    };
  }, []);

  async function signIn() {
    if (busy || disabled || attempt.current) return;
    setBusy(true);
    setError("");
    let id: string | null = null;
    try {
      id = signInAttemptId();
      attempt.current = id;
      const result = await ipc.connectGarmin(id);
      if (mounted.current && result.status === "connected") await onConnected();
    } catch (failure) {
      if (mounted.current) setError((failure as AppError).message ?? String(failure));
    } finally {
      if (attempt.current === id) attempt.current = null;
      if (mounted.current) setBusy(false);
    }
  }

  async function cancel() {
    const id = attempt.current;
    if (!id) return;
    try { await ipc.cancelGarminSignIn(id); }
    catch (failure) {
      if (mounted.current) setError((failure as AppError).message ?? String(failure));
    }
  }

  return <div className="form">
    <p className="muted footnote">Sign in on Garmin’s website in a separate window. TrainerPro stores session tokens in your system credential manager.</p>
    {error && <p className="connection-error" role="alert">{error}</p>}
    {busy && <p className="muted" role="status">Complete sign-in in the Garmin window.</p>}
    <div className="row gap">
      <button className="primary" type="button" disabled={busy || disabled} onClick={() => void signIn()}>
        {busy ? "Connecting…" : "Sign in to Garmin"}
      </button>
      {busy && <button type="button" onClick={() => void cancel()}>Cancel sign-in</button>}
    </div>
  </div>;
}
