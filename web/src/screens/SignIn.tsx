import { useMutation, useQueryClient } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import { createPasskey, getPasskey, guessDeviceName, passkeysSupported } from "@/lib/webauthn";
import type { AuthState } from "@/types/AuthState";
import type { Ceremony } from "@/types/Ceremony";

/** Passkeys only. The first one takes a one-time setup code from the server (`quietdeck-server setup-code`). */
export function SignInScreen({ setupNeeded }: { setupNeeded: boolean }) {
  const qc = useQueryClient();
  const [code, setCode] = useState("");
  const [device, setDevice] = useState(guessDeviceName);
  const [registering, setRegistering] = useState(setupNeeded);

  const done = (state: AuthState) => qc.setQueryData(["auth"], state);

  const signIn = useMutation({
    mutationFn: async () => {
      const c = await api<Ceremony>("/api/auth/login/start", { method: "POST" });
      const credential = await getPasskey(c.options);
      return api<AuthState>("/api/auth/login/finish", { method: "POST", body: { ceremony: c.ceremony, credential } });
    },
    onSuccess: done,
  });

  const register = useMutation({
    mutationFn: async () => {
      const c = await api<Ceremony>("/api/auth/register/start", {
        method: "POST",
        body: { setup_code: code.trim(), device_name: device.trim() },
      });
      const credential = await createPasskey(c.options);
      return api<AuthState>("/api/auth/register/finish", {
        method: "POST",
        body: { ceremony: c.ceremony, credential },
      });
    },
    onSuccess: done,
  });

  const error = (registering ? register.error : signIn.error)?.message;
  const cancelled = error?.includes("NotAllowedError") || error?.includes("not allowed");

  return (
    <main className="mx-auto flex min-h-dvh max-w-sm flex-col justify-center px-4 py-16">
      <h1 className="mb-1 text-2xl">Quietdeck</h1>
      {!passkeysSupported() ? (
        <p className="text-muted">This browser can’t use passkeys. Open the deck in Safari, Chrome, Edge or Firefox.</p>
      ) : registering ? (
        <form
          className="mt-6 flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            register.mutate();
          }}
        >
          <p className="text-muted">
            Quietdeck has no passwords. You sign in with a passkey, and adding one without being signed in takes a
            one-time code that proves you run this server. Run{" "}
            <code className="font-mono text-sm text-ink">quietdeck-server setup-code</code> where Quietdeck runs (in
            its container), and type the code here. It works once, for 15 minutes.
          </p>
          <label className="flex flex-col gap-1.5 text-sm text-muted">
            Setup code
            <Input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              autoComplete="one-time-code"
              autoCapitalize="characters"
              spellCheck={false}
              className="font-mono tracking-wider"
              required
              aria-invalid={register.isError || undefined}
            />
          </label>
          <label className="flex flex-col gap-1.5 text-sm text-muted">
            This device
            <Input value={device} onChange={(e) => setDevice(e.target.value)} maxLength={60} required />
          </label>
          <Button type="submit" variant="primary" loading={register.isPending}>
            <KeyRound aria-hidden />
            Create passkey
          </Button>
          {!setupNeeded && (
            <Button type="button" variant="quiet" onClick={() => setRegistering(false)}>
              Sign in with an existing passkey
            </Button>
          )}
        </form>
      ) : (
        <div className="mt-6 flex flex-col gap-4">
          <Button variant="primary" loading={signIn.isPending} onClick={() => signIn.mutate()}>
            <KeyRound aria-hidden />
            Sign in with a passkey
          </Button>
          <Button variant="quiet" onClick={() => setRegistering(true)}>
            Add this device with a setup code
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-4 text-sm text-danger">
          {cancelled ? "The passkey prompt was closed. Try again when you’re ready." : error}
        </p>
      )}
    </main>
  );
}
