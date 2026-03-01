import { createBrowserRouter } from "react-router-dom";
import { AppLayout } from "./layout/AppLayout";
import { LoginPage } from "./pages/LoginPage";
import { DashboardPage } from "./pages/DashboardPage";
import { IssuesPage } from "./pages/IssuesPage";
import { DocumentsPage } from "./pages/DocumentsPage";
import { PatchesPage } from "./pages/PatchesPage";
import { SettingsPage } from "./pages/SettingsPage";
import { IssueDetailPage } from "./pages/IssueDetailPage";
import { JobLogPage } from "./pages/JobLogPage";
import { PatchDetailPage } from "./pages/PatchDetailPage";
import { DocumentDetailPage } from "./pages/DocumentDetailPage";
import { NotificationsPage } from "./pages/NotificationsPage";

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
        element: <DashboardPage />,
      },
      {
        path: "issues",
        element: <IssuesPage />,
      },
      {
        path: "issues/:issueId",
        element: <IssueDetailPage />,
      },
      {
        path: "issues/:issueId/jobs/:jobId/logs",
        element: <JobLogPage />,
      },
      {
        path: "documents",
        element: <DocumentsPage />,
      },
      {
        path: "documents/:documentId",
        element: <DocumentDetailPage />,
      },
      {
        path: "patches",
        element: <PatchesPage />,
      },
      {
        path: "patches/:patchId",
        element: <PatchDetailPage />,
      },
      {
        path: "notifications",
        element: <NotificationsPage />,
      },
      {
        path: "settings",
        element: <SettingsPage />,
      },
    ],
  },
]);
