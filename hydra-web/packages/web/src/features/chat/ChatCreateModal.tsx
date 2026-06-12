import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Button, Icons, Kbd, Picker, PickerRow } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { useRepositories } from "../../hooks/useRepositories";
import { useFormModal } from "../../hooks/useFormModal";
import {
  readChatCreateDefaults,
  writeChatCreateDefaults,
} from "./chatCreateDefaults";
import styles from "./ChatCreateModal.module.css";

type PickerKey = "agent" | "repo" | null;

interface ChatCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function ChatCreateModal({ open, onClose }: ChatCreateModalProps) {
  const navigate = useNavigate();
  const { data: agents } = useAgents();
  const { data: repos } = useRepositories();

  const [agentName, setAgentName] = useState<string | null>(null);
  const [repoName, setRepoName] = useState<string | null>(null);
  const [picker, setPicker] = useState<PickerKey>(null);

  // On every open transition, re-seed the pickers from the persisted defaults.
  // We deliberately don't pre-seed on mount so a long-lived provider doesn't
  // hold stale defaults between opens.
  const wasOpenRef = useRef(false);
  useEffect(() => {
    const justOpened = open && !wasOpenRef.current;
    wasOpenRef.current = open;
    if (!justOpened) return;
    const defaults = readChatCreateDefaults();
    setAgentName(defaults?.agentName ?? null);
    setRepoName(defaults?.repoName ?? null);
    setPicker(null);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (picker) {
          setPicker(null);
        } else {
          onClose();
        }
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose, picker]);

  const { mutation, handleClose, handleKeyDown, isPending } = useFormModal<
    { agentName: string | null; repoName: string | null },
    Conversation
  >({
    mutationFn: ({ agentName: a, repoName: r }) =>
      apiClient.createConversation({
        ...(a && { agent_name: a }),
        ...(r && { session_settings: { repo_name: r } }),
      }),
    invalidateKeys: [["conversations"]],
    successMessage: (data) => `Conversation ${data.conversation_id} created`,
    onSuccess: (conversation) => {
      writeChatCreateDefaults({ agentName, repoName });
      onClose();
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const handleSubmit = useCallback(() => {
    mutation.mutate({ agentName, repoName });
  }, [agentName, repoName, mutation]);

  const requestClose = useCallback(() => {
    handleClose(onClose);
  }, [handleClose, onClose]);

  const onSubmitKeyDown = (e: KeyboardEvent<HTMLDivElement>) =>
    handleKeyDown(e, handleSubmit);

  const isMac =
    typeof navigator !== "undefined" && navigator.platform.includes("Mac");

  if (!open) return null;

  const agentEntries = agents ?? [];
  const repoEntries = repos ?? [];

  return (
    <div
      className={styles.backdrop}
      onClick={(e) => {
        if (e.target === e.currentTarget) requestClose();
      }}
      data-testid="chat-create-backdrop"
    >
      <div
        className={styles.modal}
        role="dialog"
        aria-modal="true"
        aria-label="Start new chat"
        data-testid="chat-create-modal"
        onKeyDown={onSubmitKeyDown}
      >
        <div className={styles.head}>
          <div className={styles.headLeft}>
            <span className={styles.headIcon}>
              <Icons.IconChat size={16} />
            </span>
            <span className={styles.headTitle}>New chat</span>
          </div>
          <button
            type="button"
            className={styles.close}
            onClick={requestClose}
            aria-label="Close"
          >
            <Icons.IconX size={14} />
          </button>
        </div>

        <div className={styles.body}>
          <p className={styles.intro}>
            Pick an agent and a repository for the chat. Both are optional —
            your last selection is remembered.
          </p>

          <div className={styles.pickers}>
            <Picker
              data-testid="chat-create-agent-picker"
              label="Agent"
              open={picker === "agent"}
              onToggle={() => setPicker(picker === "agent" ? null : "agent")}
              wide
              value={
                agentName ? (
                  <span className={styles.pillContent}>
                    <Avatar name={agentName} kind="agent" size="md" />
                    <span>{agentName}</span>
                  </span>
                ) : (
                  <span className={styles.pillEmpty}>Unassigned</span>
                )
              }
            >
              <PickerRow
                active={!agentName}
                onClick={() => {
                  setAgentName(null);
                  setPicker(null);
                }}
              >
                <span className={styles.pillEmpty}>Unassigned</span>
                <span className={styles.popSpacer} />
              </PickerRow>
              {agentEntries.length > 0 && (
                <>
                  <div className={styles.popSection}>Agents</div>
                  {agentEntries.map((a) => (
                    <PickerRow
                      key={a.name}
                      active={agentName === a.name}
                      onClick={() => {
                        setAgentName(a.name);
                        setPicker(null);
                      }}
                    >
                      <Avatar name={a.name} kind="agent" size="md" />
                      <span>{a.name}</span>
                      <span className={styles.popSpacer} />
                    </PickerRow>
                  ))}
                </>
              )}
            </Picker>

            <Picker
              data-testid="chat-create-repo-picker"
              label="Repository"
              open={picker === "repo"}
              onToggle={() => setPicker(picker === "repo" ? null : "repo")}
              wide
              value={
                repoName ? (
                  <code className={styles.pillCode}>{repoName}</code>
                ) : (
                  <span className={styles.pillEmpty}>None</span>
                )
              }
            >
              <PickerRow
                active={!repoName}
                onClick={() => {
                  setRepoName(null);
                  setPicker(null);
                }}
              >
                <span className={styles.pillEmpty}>None</span>
                <span className={styles.popSpacer} />
              </PickerRow>
              {repoEntries.length === 0 ? (
                <div className={styles.popEmpty}>No repositories</div>
              ) : (
                repoEntries.map((r) => (
                  <PickerRow
                    key={r.name}
                    active={repoName === r.name}
                    onClick={() => {
                      setRepoName(r.name);
                      setPicker(null);
                    }}
                  >
                    <Icons.IconRepo size={14} />
                    <code className={styles.popCode}>{r.name}</code>
                    <span className={styles.popSpacer} />
                  </PickerRow>
                ))
              )}
            </Picker>
          </div>
        </div>

        <div className={styles.foot}>
          <span className={styles.footSpacer} />
          <span className={styles.footHint}>
            <Kbd>{isMac ? "⌘" : "Ctrl"}</Kbd>
            <Kbd>↵</Kbd> submit
          </span>
          <Button variant="ghost" size="sm" onClick={requestClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={handleSubmit}
            disabled={isPending}
            data-testid="chat-create-submit"
          >
            <Icons.IconPlus size={14} />
            {isPending ? "Creating…" : "Start chat"}
          </Button>
        </div>
      </div>
    </div>
  );
}
