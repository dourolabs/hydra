/// <reference types="vite/client" />
import { Suspense } from "react";
import { RouterProvider } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import "@hydra/ui/style.css";
import "@hydra/ui/theme/global.css";
import { AuthProvider } from "./features/auth/AuthContext";
import { ToastProvider } from "./features/toast/ToastContext";
import { router } from "./router";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
});

// Dev-only: expose the QueryClient on `window` so Playwright specs can drive
// cache invalidation directly. Used by the `@chat:activity-status` e2e spec
// to invalidate `["sessionEvents", sid]` after POST-ing a synthesised
// SessionEvent to the mock-server's `POST /v1/dev/sessions/:id/events` test
// seam — the mock-server's `/v1/events` SSE filter currently rejects
// `session_event_created` events because it compares against the full event
// name instead of the entity-category prefix the frontend sends as
// `types=...,sessions,...`, so real-SSE invalidation can't reach the browser
// without a separate mock-server fix. Tracked as a follow-up.
// Guarded so it never ships in production bundles (Vite drops the entire
// `if` block when `import.meta.env.DEV` is statically false).
if (import.meta.env.DEV) {
  (window as unknown as { __hydraQueryClient?: QueryClient }).__hydraQueryClient =
    queryClient;
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <ToastProvider>
          <Suspense fallback={<Spinner />}>
            <RouterProvider router={router} />
          </Suspense>
        </ToastProvider>
      </AuthProvider>
    </QueryClientProvider>
  );
}
