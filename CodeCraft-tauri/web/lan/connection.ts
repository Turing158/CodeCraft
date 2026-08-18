/** Token and live-stream state for the LAN console.
 *
 * The reconnect delays and the auth transitions live here as pure functions so
 * the behaviour is testable without a network or a browser.
 */

export type ConnectionPhase =
  | "checking"
  | "token"
  | "connecting"
  | "live"
  | "reconnecting";

export interface ConnectionState {
  phase: ConnectionPhase;
  attempt: number;
  error: string | null;
}

export const initialConnectionState: ConnectionState = {
  phase: "checking",
  attempt: 0,
  error: null,
};

export const BASE_RECONNECT_DELAY_MS = 500;
export const MAX_RECONNECT_DELAY_MS = 15_000;

/** Exponential backoff, capped so a long outage still retries twice a minute. */
export const reconnectDelay = (attempt: number): number => {
  const normalized = Math.max(1, Math.floor(attempt));
  const delay = BASE_RECONNECT_DELAY_MS * 2 ** (normalized - 1);
  return Math.min(MAX_RECONNECT_DELAY_MS, delay);
};

export type ConnectionEvent =
  | { type: "sessionMissing" }
  | { type: "tokenRejected"; message: string }
  | { type: "tokenAccepted" }
  | { type: "streamOpen" }
  | { type: "streamLost" }
  | { type: "signedOut" };

export const nextConnectionState = (
  state: ConnectionState,
  event: ConnectionEvent,
): ConnectionState => {
  switch (event.type) {
    case "sessionMissing":
    case "signedOut":
      return { phase: "token", attempt: 0, error: null };
    case "tokenRejected":
      return { phase: "token", attempt: 0, error: event.message };
    case "tokenAccepted":
      return { phase: "connecting", attempt: 0, error: null };
    case "streamOpen":
      return { phase: "live", attempt: 0, error: null };
    case "streamLost":
      // A drop before the first successful token exchange means the session is
      // gone, so ask for the token again instead of retrying forever.
      if (state.phase === "token") return state;
      return {
        phase: "reconnecting",
        attempt: state.attempt + 1,
        error: null,
      };
    default:
      return state;
  }
};

const PHASE_LABELS: Record<ConnectionPhase, string> = {
  checking: "正在检查登录状态",
  token: "需要访问令牌",
  connecting: "连接中",
  live: "已连接",
  reconnecting: "重新连接中",
};

export const connectionLabel = (state: ConnectionState): string =>
  state.phase === "reconnecting"
    ? PHASE_LABELS.reconnecting + " (" + state.attempt + ")"
    : PHASE_LABELS[state.phase];

export const isAuthenticated = (state: ConnectionState): boolean =>
  state.phase === "connecting" ||
  state.phase === "live" ||
  state.phase === "reconnecting";

/** Tokens are 32 hex characters; checking locally avoids a pointless round trip
 * and a wasted brute-force attempt. */
export const tokenLooksValid = (token: string): boolean =>
  /^[0-9a-zA-Z]{16,64}$/.test(token.trim());
