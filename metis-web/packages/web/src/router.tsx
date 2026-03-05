import { createBrowserRouter } from "react-router-dom";
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
        element: <DashboardPage />,
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
        path: "patches/:patchId",
        element: <PatchDetailPage />,
      },
      {
        path: "settings",
        element: <SettingsPage />,
      },
    ],
  },
]);
