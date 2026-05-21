export const UI_VERSION = "0.0.1";

// Theme
import "./theme/tokens.css";

// Components
export { Button } from "./components/Button";
export type { ButtonProps } from "./components/Button";

export { Input } from "./components/Input";
export type { InputProps } from "./components/Input";

export { Textarea } from "./components/Textarea";
export type { TextareaProps } from "./components/Textarea";

export { Badge } from "./components/Badge";
export type { BadgeProps, BadgeStatus } from "./components/Badge";

export { Select } from "./components/Select";
export type { SelectProps, SelectOption } from "./components/Select";

export { Spinner } from "./components/Spinner";
export type { SpinnerProps } from "./components/Spinner";

export { Panel } from "./components/Panel";
export type { PanelProps } from "./components/Panel";

export { TreeView } from "./components/TreeView";
export type { TreeViewProps, TreeNode } from "./components/TreeView";

export { Tabs } from "./components/Tabs";
export type { TabsProps, Tab } from "./components/Tabs";

export { Modal } from "./components/Modal";
export type { ModalProps } from "./components/Modal";

export { Picker, PickerRow } from "./components/Picker";
export type { PickerProps, PickerRowProps } from "./components/Picker";

export { Tooltip } from "./components/Tooltip";
export type { TooltipProps, TooltipTrigger } from "./components/Tooltip";

export { Avatar } from "./components/Avatar";
export type { AvatarProps } from "./components/Avatar";

export { LogViewer } from "./components/LogViewer";
export type { LogViewerProps } from "./components/LogViewer";

export { MarkdownViewer } from "./components/MarkdownViewer";
export type { MarkdownViewerProps } from "./components/MarkdownViewer";

export { Toast } from "./components/Toast";
export type { ToastProps, ToastVariant } from "./components/Toast";

export { SessionStatusIndicator } from "./components/SessionStatusIndicator";
export type { SessionStatusIndicatorProps, SessionSummary, SessionStatus } from "./components/SessionStatusIndicator";

export { DiffViewer } from "./components/DiffViewer";
export type { DiffViewerProps } from "./components/DiffViewer";

export { CopyButton, fallbackCopyText } from "./components/CopyButton";
export type { CopyButtonProps } from "./components/CopyButton";

export { Chip } from "./components/Chip";
export type { ChipProps, ChipTone } from "./components/Chip";

export { TypeChip, issueTypeDisplayLabel } from "./components/TypeChip";
export type { TypeChipProps, IssueType } from "./components/TypeChip";

export { Kbd } from "./components/Kbd";
export type { KbdProps } from "./components/Kbd";

export { HydraMark, HYDRA_VARIANTS } from "./components/HydraMark";
export type { HydraMarkProps, HydraVariant } from "./components/HydraMark";

export * as Icons from "./components/Icon";

// Hooks
export { useKeyboardClick } from "./hooks/useKeyboardClick";
export type { KeyboardClickProps } from "./hooks/useKeyboardClick";

export { ErrorBoundary } from "./components/ErrorBoundary";
export type { ErrorBoundaryProps } from "./components/ErrorBoundary";
