import { createBrowserRouter } from "react-router-dom";
import { ErrorBoundary } from "@metis/ui";
import { AppLayout } from "./layout/AppLayout";
import { LoginPage } from "./pages/LoginPage";
import { DashboardPage } from "./pages/DashboardPage";
import { DocumentsPage } from "./pages/DocumentsPage";
import { SettingsPage } from "./pages/SettingsPage";
import { IssueDetailPage } from "./pages/IssueDetailPage";
import { JobLogPage } from "./pages/JobLogPage";
import { PatchDetailPage } from "./pages/PatchDetailPage";
import { DocumentDetailPage } from "./pages/DocumentDetailPage";

export const router = createBrowserRouter([
  {
    path: "/login",
    element: <LoginPage />,
  },
  {
    path: "/",
    element: <AppLayout />,
    children: [
      {
        index: true,
        element: (
          <ErrorBoundary>
            <DashboardPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "issues/:issueId",
        element: (
          <ErrorBoundary>
            <IssueDetailPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "issues/:issueId/jobs/:jobId/logs",
        element: (
          <ErrorBoundary>
            <JobLogPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "documents",
        element: (
          <ErrorBoundary>
            <DocumentsPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "documents/:documentId",
        element: (
          <ErrorBoundary>
            <DocumentDetailPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "patches/:patchId",
        element: (
          <ErrorBoundary>
            <PatchDetailPage />
          </ErrorBoundary>
        ),
      },
      {
        path: "settings",
        element: (
          <ErrorBoundary>
            <SettingsPage />
          </ErrorBoundary>
        ),
      },
    ],
  },
]);
