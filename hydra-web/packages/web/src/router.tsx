import { createBrowserRouter } from "react-router-dom";
import { ErrorBoundary } from "@hydra/ui";
import { AppLayout } from "./layout/AppLayout";

// Silences React Router's "No HydrateFallback element provided" warning. The
// app is an SPA and routes are lazy-loaded, so this only renders during the
// brief moment before the initial chunk resolves — an empty placeholder is
// enough and avoids any visible flash on fast loads.
const hydrateFallbackElement = <div />;

export const router = createBrowserRouter([
  {
    path: "/login",
    hydrateFallbackElement,
    lazy: () =>
      import("./pages/LoginPage").then((m) => ({ Component: m.LoginPage })),
  },
  {
    path: "/",
    element: <AppLayout />,
    hydrateFallbackElement,
    children: [
      {
        index: true,
        lazy: () =>
          import("./pages/IssuesListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.IssuesListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "issues/:issueId",
        lazy: () =>
          import("./pages/IssueDetailPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.IssueDetailPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "issues/:issueId/sessions/:sessionId/logs",
        lazy: () =>
          import("./pages/SessionLogPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.SessionLogPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "sessions",
        lazy: () =>
          import("./pages/SessionsListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.SessionsListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "sessions/:sessionId",
        lazy: () =>
          import("./pages/SessionLogPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.SessionLogPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "chat",
        lazy: () =>
          import("./pages/ChatListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.ChatListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "chat/:conversationId",
        lazy: () =>
          import("./pages/ChatPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.ChatPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "documents",
        lazy: () =>
          import("./pages/DocumentsPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.DocumentsPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "documents/:documentId",
        lazy: () =>
          import("./pages/DocumentDetailPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.DocumentDetailPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "patches",
        lazy: () =>
          import("./pages/PatchesListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.PatchesListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "patches/:patchId",
        lazy: () =>
          import("./pages/PatchDetailPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.PatchDetailPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "agents",
        lazy: () =>
          import("./pages/AgentsPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.AgentsPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "secrets",
        lazy: () =>
          import("./pages/SecretsPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.SecretsPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "repositories",
        lazy: () =>
          import("./pages/RepositoriesPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.RepositoriesPage />
              </ErrorBoundary>
            ),
          })),
      },
    ],
  },
]);
