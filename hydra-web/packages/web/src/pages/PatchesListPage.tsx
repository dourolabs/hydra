import { PatchesView } from "../features/patches/view/PatchesView";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";

export function PatchesListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Patches");
  return <PatchesView />;
}
