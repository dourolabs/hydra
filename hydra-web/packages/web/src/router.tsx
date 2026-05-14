import { createBrowserRouter } from "react-router-dom";
import { ErrorBoundary } from "@hydra/ui";
import { AppLayout } from "./layout/AppLayout";

export const router = createBrowserRouter([
  {
    path: "/login",
    lazy: () =>
      import("./pages/LoginPage").then((m) => ({ Component: m.LoginPage })),
  },
  {
    path: "/",
    element: <AppLayout />,
    children: [
      {
        index: true,
        lazy: () =>
          import("./pages/DashboardPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.DashboardPage />
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
        path: "settings",
        lazy: () =>
          import("./pages/SettingsPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.SettingsPage />
              </ErrorBoundary>
            ),
          })),
      },
    ],
  },
]);
