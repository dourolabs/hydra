import { createBrowserRouter } from "react-router-dom";
import { AppLayout } from "./layout/AppLayout";
import { LoginPage } from "./pages/LoginPage";
import { DashboardPage } from "./pages/DashboardPage";
import { IssuesPage } from "./pages/IssuesPage";
import { IssueDetailPage } from "./pages/IssueDetailPage";
import { JobLogPage } from "./pages/JobLogPage";
import { DocumentsPage } from "./pages/DocumentsPage";
import { PatchesPage } from "./pages/PatchesPage";
import { PatchDetailPage } from "./pages/PatchDetailPage";
import { SettingsPage } from "./pages/SettingsPage";

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
        path: "patches",
        element: <PatchesPage />,
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
