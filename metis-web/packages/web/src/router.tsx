import { createBrowserRouter } from "react-router-dom";
import { ErrorBoundary } from "@metis/ui";
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
        path: "issues/:issueId/jobs/:jobId/logs",
        lazy: () =>
          import("./pages/JobLogPage").then((m) => ({
            element: (
              <ErrorBoundary>
                <m.JobLogPage />
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
