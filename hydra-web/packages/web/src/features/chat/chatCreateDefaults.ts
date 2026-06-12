const STORAGE_KEY = "hydra:v1:chat-create:defaults";

export interface ChatCreateDefaults {
  agentName: string | null;
  repoName: string | null;
}

export function readChatCreateDefaults(): ChatCreateDefaults | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    const agentName =
      typeof parsed.agentName === "string" ? parsed.agentName : null;
    const repoName =
      typeof parsed.repoName === "string" ? parsed.repoName : null;
    return { agentName, repoName };
  } catch {
    return null;
  }
}

export function writeChatCreateDefaults(value: ChatCreateDefaults) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    // ignore
  }
}
