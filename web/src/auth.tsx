import { useState, type FormEvent } from "react";
import { setToken, verifyToken } from "./api/client";

/**
 * The backend's human-facing auth is a single static bearer token, no
 * sessions (ADR-009 §11.11) — there's nothing for the browser to log in
 * to. This gate is the SPA's one piece of state the fixture-backed design
 * didn't need: paste the token once, verified against a real endpoint
 * before it's trusted, then stored for next time.
 */
export function TokenGate({ onAuthenticated }: { onAuthenticated: () => void }) {
  const [value, setValue] = useState("");
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const candidate = value.trim();
    if (!candidate) return;
    setChecking(true);
    setError(null);
    try {
      const ok = await verifyToken(candidate);
      if (!ok) {
        setError("Token rejected — check it against `monitra start`'s output or `monitra web`'s.");
        return;
      }
      setToken(candidate);
      onAuthenticated();
    } catch {
      setError("Could not reach the daemon — is it running?");
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="boot-screen">
      <span className="brand-mark">M</span>
      <form className="token-gate" onSubmit={submit}>
        <h1>Connect to Monitra</h1>
        <p>Paste the API token printed by <code>monitra start</code> or <code>monitra web</code>.</p>
        <input
          type="password"
          autoFocus
          value={value}
          onChange={(event) => setValue(event.target.value)}
          placeholder="API token"
          spellCheck={false}
          autoComplete="off"
        />
        <button className="button primary" type="submit" disabled={checking || value.trim().length === 0}>
          {checking ? "Checking…" : "Connect"}
        </button>
        {error && <p className="token-gate-error">{error}</p>}
      </form>
    </div>
  );
}
