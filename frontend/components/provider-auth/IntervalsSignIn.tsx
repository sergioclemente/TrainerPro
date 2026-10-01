import { FormEvent, useState } from "react";
import { AppError, ipc } from "../../ipc";
import type { SignInProps } from "./index";

export default function IntervalsSignIn({ disabled, onConnected }: SignInProps) {
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function connect(event: FormEvent) {
    event.preventDefault();
    if (busy || disabled || !apiKey.trim()) return;
    setBusy(true);
    setError("");
    const submittedKey = apiKey;
    setApiKey("");
    try {
      await ipc.connectIntervalsIcu(submittedKey);
      await onConnected();
    } catch (failure) { setError((failure as AppError).message ?? String(failure)); }
    finally { setBusy(false); }
  }
  return <form className="form" onSubmit={(event) => void connect(event)}>
    {error && <p className="connection-error" role="alert">{error}</p>}
    <p className="muted">Create a personal API key in Intervals.icu under Settings → Developer Settings. TrainerPro stores it in your system credential manager.</p>
    <label>Personal API key<input type="password" value={apiKey} autoComplete="off" spellCheck={false} disabled={busy || disabled} onChange={(event) => setApiKey(event.target.value)} /></label>
    <button className="primary" disabled={busy || disabled || !apiKey.trim()}>{busy ? "Connecting…" : "Connect and sync"}</button>
  </form>;
}
