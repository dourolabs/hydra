import { useAuth } from "./useAuth";

/**
 * Returns the authenticated user's username, or null if not logged in.
 */
export function useUsername(): string | null {
  const { user } = useAuth();
  if (!user) return null;
  const actor = user.actor;
  switch (actor.type) {
    case "user":
      return actor.username;
    case "session":
    case "issue":
      return actor.creator;
    case "service":
      return actor.service_name;
  }
}
