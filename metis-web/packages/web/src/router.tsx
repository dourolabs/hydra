import { createBrowserRouter } from "react-router-dom";
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
            Component: m.DashboardPage,
          })),
      },
      {
        path: "issues/:issueId",
        lazy: () =>
          import("./pages/IssueDetailPage").then((m) => ({
            Component: m.IssueDetailPage,
          })),
      },
      {
        path: "issues/:issueId/jobs/:jobId/logs",
        lazy: () =>
          import("./pages/JobLogPage").then((m) => ({
            Component: m.JobLogPage,
          })),
      },
      {
        path: "documents",
        lazy: () =>
          import("./pages/DocumentsPage").then((m) => ({
            Component: m.DocumentsPage,
          })),
      },
      {
        path: "documents/:documentId",
        lazy: () =>
          import("./pages/DocumentDetailPage").then((m) => ({
            Component: m.DocumentDetailPage,
          })),
      },
      {
        path: "patches/:patchId",
        lazy: () =>
          import("./pages/PatchDetailPage").then((m) => ({
            Component: m.PatchDetailPage,
          })),
      },
      {
        path: "settings",
        lazy: () =>
          import("./pages/SettingsPage").then((m) => ({
            Component: m.SettingsPage,
          })),
      },
    ],
  },
]);
