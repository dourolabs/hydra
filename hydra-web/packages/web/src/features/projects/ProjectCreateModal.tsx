import { Modal } from "@hydra/ui";
import { useUsername } from "../auth/useUsername";
import { ProjectForm } from "./ProjectForm";

interface ProjectCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function ProjectCreateModal({ open, onClose }: ProjectCreateModalProps) {
  // Rendered only under `AppLayout`'s auth guard, so the user is non-null here.
  const username = useUsername();
  if (!username) return null;
  return (
    <Modal open={open} onClose={onClose} title="New project">
      <ProjectForm creator={username} onClose={onClose} />
    </Modal>
  );
}
