import { apiClient } from "./client";

/**
 * Mint the proxy cookie bound to a conversation's currently-active session.
 *
 * Set-Cookie lands on the response and is HttpOnly so the browser attaches it
 * to subsequent `<port>-<conv-id>.proxy.<host>` requests but page JS cannot
 * read it. Callers must `await` this before `window.open(...)` so the new tab
 * loads with the cookie already in place.
 *
 * 409 from the server is the "conversation is idle, no active session" signal
 * — surface "send a message to resume" rather than retrying.
 */
export async function mintConversationProxyCookie(
  conversationId: string,
): Promise<void> {
  await apiClient.mintConversationProxyAuth(conversationId);
}

/**
 * Hostname of the proxy subdomain root.
 *
 * The server's `config.hydra.proxy_host` (default `proxy.localhost`) defines
 * the actual suffix; the simplest reliable derivation from the browser is to
 * prepend `proxy.` to whatever main host the SPA is hosted on. Same rule as
 * the cookie's `Domain=.proxy.<host>` scope chosen by the server.
 */
export function proxyHostFor(mainHost: string): string {
  return `proxy.${mainHost}`;
}

/**
 * Build the per-target proxy URL for opening in a new tab.
 *
 * `targetLabel` is either a `c-…` conversation id (the default form) or an
 * `s-…` session id (for direct-session debugging via the copy affordance).
 */
export function buildProxyUrl(options: {
  port: number;
  targetLabel: string;
  readyPath?: string | null;
  mainHost: string;
  protocol?: string;
}): string {
  const path = options.readyPath ?? "/";
  const protocol = options.protocol ?? "https:";
  const host = proxyHostFor(options.mainHost);
  return `${protocol}//${options.port}-${options.targetLabel}.${host}${path}`;
}
