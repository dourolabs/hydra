import { createBrowserRouter, Navigate } from "react-router-dom";
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
    lazy: () => import("./pages/LoginPage").then((m) => ({ Component: m.LoginPage })),
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
        path: "triggers",
        lazy: () =>
          import("./pages/TriggersListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.TriggersListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "triggers/:triggerId",
        lazy: () =>
          import("./pages/TriggerDetailPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.TriggerDetailPage />
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
      {
        path: "projects",
        lazy: () =>
          import("./pages/ProjectsListPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.ProjectsListPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "analytics",
        element: <Navigate to="/analytics/throughput" replace />,
      },
      {
        path: "analytics/throughput",
        lazy: () =>
          import("./pages/AnalyticsThroughputPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.AnalyticsThroughputPage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "analytics/token-usage",
        lazy: () =>
          import("./pages/AnalyticsTokenUsagePage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.AnalyticsTokenUsagePage />
              </ErrorBoundary>
            ),
          })),
      },
      {
        path: "*",
        lazy: () =>
          import("./pages/NotFoundPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.NotFoundPage />
              </ErrorBoundary>
            ),
          })),
      },
    ],
  },
]);
