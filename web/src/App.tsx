import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Header } from "@/components/Header";
import { api, setSignedOutHandler } from "@/lib/api";
import { useDeckEvents } from "@/lib/events";
import { usePath } from "@/lib/route";
import { HistoryScreen } from "@/screens/History";
import { InboxScreen } from "@/screens/Inbox";
import { LaterScreen } from "@/screens/Later";
import { RulesScreen } from "@/screens/Rules";
import { SessionScreen } from "@/screens/Session";
import { SessionPageScreen } from "@/screens/SessionPage";
import { SessionsScreen } from "@/screens/Sessions";
import { SettingsScreen } from "@/screens/Settings";
import { SignInScreen } from "@/screens/SignIn";
import type { AuthState } from "@/types/AuthState";

export function App() {
  const qc = useQueryClient();
  const auth = useQuery({
    queryKey: ["auth"],
    queryFn: () => api<AuthState>("/api/auth/state"),
  });
  const signedIn = auth.data?.signed_in === true;

  useEffect(
    () =>
      setSignedOutHandler(
        () => void qc.invalidateQueries({ queryKey: ["auth"] }),
      ),
    [qc],
  );
  useDeckEvents(signedIn);

  if (auth.isPending) return null;
  if (auth.isError) {
    return (
      <main className="mx-auto max-w-md px-4 py-24 text-muted">
        <h1 className="mb-2 text-xl text-ink">The deck isn’t answering</h1>
        <p>{auth.error.message}</p>
      </main>
    );
  }
  if (!signedIn) return <SignInScreen setupNeeded={auth.data.setup_needed} />;
  return (
    <>
      <Header />
      <Screen />
    </>
  );
}

function Screen() {
  const path = usePath();
  if (path.startsWith("/history")) return <HistoryScreen />;
  if (path.startsWith("/later")) return <LaterScreen />;
  if (path.startsWith("/rules")) return <RulesScreen />;
  if (path.startsWith("/settings")) return <SettingsScreen />;
  if (path.startsWith("/sessions")) {
    // /sessions, /sessions/<machine>/<session ID>, /sessions/<machine>/<session ID>/page
    const [, , node, sessionId, page] = path.split("/").map(decodeURIComponent);
    if (node && sessionId && page === "page") return <SessionPageScreen key={`${node}/${sessionId}`} node={node} sessionId={sessionId} />;
    if (node && sessionId) return <SessionScreen key={`${node}/${sessionId}`} node={node} sessionId={sessionId} />;
    return <SessionsScreen />;
  }
  return <InboxScreen />;
}
