import { Modal } from "@hydra/ui";
import { ProjectEditor } from "./ProjectEditor";
import { useUsername } from "../auth/useUsername";
import largeModalStyles from "../../components/LargeModal.module.css";

interface ProjectCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function ProjectCreateModal({ open, onClose }: ProjectCreateModalProps) {
  const username = useUsername() ?? "unknown";
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
