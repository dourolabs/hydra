import { SessionsView } from "../features/sessions/view/SessionsView";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";

export function SessionsListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Sessions");
  return <SessionsView />;
}
