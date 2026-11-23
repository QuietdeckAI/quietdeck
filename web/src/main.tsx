import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/App";
import { TabBadge } from "@/components/TabBadge";
import { DeckError } from "@/lib/api";
import "@/app.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The event stream says when something changed, so nothing needs polling or refetching on focus.
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      retry: (count, error) => !(error instanceof DeckError && error.status < 500) && count < 2,
    },
  },
});

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
      <TabBadge />
    </QueryClientProvider>
  </StrictMode>,
);
