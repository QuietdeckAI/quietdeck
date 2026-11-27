import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Header } from "@/components/Header";
import { api, setSignedOutHandler } from "@/lib/api";
import { useDeckEvents } from "@/lib/events";
import { usePath } from "@/lib/route";
import { ThreadsScreen } from "@/screens/Threads";
import { HistoryScreen } from "@/screens/History";
import { InboxScreen } from "@/screens/Inbox";
import { MailScreen } from "@/screens/Mail";
import { useHomeEndInTextBoxes } from "@/lib/homeEnd";
import { LaterScreen } from "@/screens/Later";
import { RulesScreen } from "@/screens/Rules";
import { SessionScreen } from "@/screens/Session";
import { SessionPageScreen } from "@/screens/SessionPage";
import { SessionsScreen } from "@/screens/Sessions";
import { SettingsScreen } from "@/screens/Settings";
import { SignInScreen } from "@/screens/SignIn";
import { SnapshotScreen } from "@/screens/Snapshot";
import type { AuthState } from "@/types/AuthState";
import { tabAt, useDocuments } from "@/lib/documents";

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
  useHomeEndInTextBoxes();

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
    // One column at least the screen's height: the header, then the screen, which can fill the rest (the Inbox does,
    // so its bar sits at the bottom with no scroll), without adding up pixel heights that rounding can upset.
    <div className="flex min-h-dvh flex-col">
      <Header />
      {/* The rest of the screen, as a column: the Inbox grows to fill it (flex-1). Every screen's root is full width
          here (w-full), so a centred `mx-auto max-w-*` root lays out as it would in a plain block. */}
      <div className="flex flex-1 flex-col [&>*]:w-full">
        <Screen />
      </div>
    </div>
  );
}

function Screen() {
  const path = usePath();
  const docs = useDocuments().data;
  const tab = tabAt(docs, path);
  if (tab) return <InboxScreen key={tab.path} tab={tab} />;
  // `/cards` was the screen's old address.
  if (path.startsWith("/threads") || path.startsWith("/cards")) return <ThreadsScreen />;
  if (path.startsWith("/history/inbox/")) {
    const id = Number(path.split("/")[3]);
    if (Number.isInteger(id)) return <SnapshotScreen key={id} id={id} />;
  }
  if (path.startsWith("/history")) return <HistoryScreen />;
  if (path.startsWith("/drafts")) return <MailScreen key="draft" kind="draft" />;
  if (path.startsWith("/sent")) return <MailScreen key="sent" kind="sent" />;
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
