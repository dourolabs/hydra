import { useParams } from "react-router-dom";
import { ExistingChatPage } from "../features/chat/ExistingChatPage";

export function ChatPage() {
  const { conversationId } = useParams<{ conversationId: string }>();
  // Key by conversationId so react-router soft-navigation between two chats
  // remounts ExistingChatPage instead of reusing it. The reused instance had
  // leaked locally-buffered optimistic events (and any other component state)
  // from the previous conversation into the new one until a hard refresh.
  const id = conversationId ?? "";
  return <ExistingChatPage key={id} conversationId={id} />;
}
