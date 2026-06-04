import { Modal } from "@hydra/ui";
import { ProjectEditor } from "./ProjectEditor";
import { useUsername } from "../auth/useUsername";
import largeModalStyles from "../../components/LargeModal.module.css";

interface ProjectCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function ProjectCreateModal({ open, onClose }: ProjectCreateModalProps) {
  // Rendered only under `AppLayout`'s auth guard, so the user is non-null here.
  const username = useUsername();
  if (!username) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      title="New project"
      className={largeModalStyles.largeModal}
    >
      <ProjectEditor creator={username} />
    </Modal>
  );
}
