import { useEffect, useRef, useState } from "react";
import { AppError, ipc } from "../../ipc";

import type { SignInProps } from "./index";

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
    const id = crypto.randomUUID();
    attempt.current = id;
    try {
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
