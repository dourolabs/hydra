import { createBrowserRouter } from "react-router-dom";
import { AppLayout } from "./layout/AppLayout";
import { LoginPage } from "./pages/LoginPage";
import { DashboardPage } from "./pages/DashboardPage";
import { IssueDetailPage } from "./pages/IssueDetailPage";
import { TaskLogPage } from "./pages/TaskLogPage";

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
        path: "issues/:issueId/tasks/:taskId/logs",
        element: <TaskLogPage />,
      },
    ],
  },
]);
