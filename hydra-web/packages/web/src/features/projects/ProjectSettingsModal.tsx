import { Modal } from "@hydra/ui";
import type { ProjectRecord } from "@hydra/api";
import { ProjectForm } from "./ProjectForm";

interface ProjectSettingsModalProps {
  open: boolean;
  onClose: () => void;
  project: ProjectRecord;
}

export function ProjectSettingsModal({
  open,
  onClose,
  project,
}: ProjectSettingsModalProps) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Project settings · ${project.project.name}`}
    >
      <ProjectForm
        projectId={project.project_id}
        initial={project.project}
        creator={project.project.creator}
        onClose={onClose}
      />
    </Modal>
  );
}
