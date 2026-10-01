import { FormEvent, useEffect, useState } from "react";
import { AppError, ipc } from "../../ipc";

import type { SignInProps } from "./index";

export default function GarminSignIn({ disabled, onConnected }: SignInProps) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [needsMfa, setNeedsMfa] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => () => { void ipc.cancelGarminSignIn().catch(() => {}); }, []);

  async function signIn(event: FormEvent) {
    event.preventDefault();
    if (busy || disabled) return;
    setBusy(true);
    setError("");
    const submittedPassword = password;
    const submittedCode = code;
    setPassword("");
    setCode("");
    try {
      if (needsMfa) {
        await ipc.completeGarminMfa(submittedCode);
        setNeedsMfa(false);
      } else {
        const result = await ipc.connectGarmin(email, submittedPassword);
        if (result.status === "needs_mfa") {
          setNeedsMfa(true);
          return;
        }
      }
      setEmail("");
      await onConnected();
    } catch (failure) {
      const appError = failure as AppError;
      setError(appError.message ?? String(failure));
      if (appError.code === "garmin_mfa_expired") setNeedsMfa(false);
    } finally { setBusy(false); }
  }

  async function cancel() {
    setBusy(true);
    try {
      await ipc.cancelGarminSignIn();
      setNeedsMfa(false);
      setCode("");
      setError("");
    } catch (failure) { setError((failure as AppError).message ?? String(failure)); }
    finally { setBusy(false); }
  }

  const blocked = busy || disabled;
  return <form className="form" onSubmit={(event) => void signIn(event)}>
    {error && <p className="connection-error" role="alert">{error}</p>}
    {needsMfa ? <label>Garmin verification code
      <input autoFocus autoComplete="one-time-code" value={code} onChange={(event) => setCode(event.target.value)} disabled={blocked} required />
    </label> : <>
      <p className="muted footnote">Sign in with your Garmin account. TrainerPro keeps session tokens in your system credential manager. Your password is used only for this sign-in.</p>
      <label>Email<input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} disabled={blocked} required /></label>
      <label>Password<input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} disabled={blocked} required /></label>
    </>}
    <div className="row gap">
      <button className="primary" type="submit" disabled={blocked || (needsMfa ? !code.trim() : !email.trim() || !password)}>
        {busy ? "Connecting…" : needsMfa ? "Verify code" : "Sign in"}
      </button>
      {needsMfa && <button type="button" disabled={blocked} onClick={() => void cancel()}>Cancel sign-in</button>}
    </div>
  </form>;
}
