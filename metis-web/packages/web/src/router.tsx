import { createBrowserRouter } from "react-router-dom";
import { AppLayout } from "./layout/AppLayout";
import { LoginPage } from "./pages/LoginPage";
import { DashboardPage } from "./pages/DashboardPage";
import { IssueDetailPage } from "./pages/IssueDetailPage";
import { JobLogPage } from "./pages/JobLogPage";
import { PatchDetailPage } from "./pages/PatchDetailPage";

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
        path: "patches/:patchId",
        element: <PatchDetailPage />,
      },
    ],
  },
]);
