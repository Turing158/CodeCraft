/** Thin fetch wrapper for the LAN console API.
 *
 * Every write carries the custom header the server requires; browsers cannot add
 * it during a cross-site form or image request, so it blocks CSRF.
 */

import type { LocalQuestionAnswer } from "../../src/question-flow";

const REQUEST_HEADER = "X-CodeCraft-Lan";

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.name = "ApiError";
  }
}

const parseError = async (response: Response): Promise<string> => {
  try {
    const body = (await response.json()) as { error?: unknown };
    if (typeof body.error === "string") return body.error;
  } catch {
    // Fall through to the generic message below.
  }
  return "请求失败 (" + response.status + ")";
};

const request = async <T>(
  path: string,
  init: RequestInit = {},
): Promise<T> => {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      [REQUEST_HEADER]: "1",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...(init.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw new ApiError(response.status, await parseError(response));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
};

export const authenticate = (token: string) =>
  request<{ allowApprovals: boolean }>("/api/auth", {
    method: "POST",
    body: JSON.stringify({ token }),
  });

export const signOut = () => request<{ ok: boolean }>("/api/logout", { method: "POST" });

export const fetchState = () => request<Record<string, unknown>>("/api/state");

export type PermissionDecision = "allow" | "allowAlways" | "deny";

export const submitPermission = (
  requestId: string,
  decision: PermissionDecision,
) =>
  request<{ ok: boolean }>("/api/claude/permission", {
    method: "POST",
    body: JSON.stringify({ requestId, decision }),
  });

export const submitQuestion = (
  requestId: string,
  answers: LocalQuestionAnswer[],
) =>
  request<{ ok: boolean }>("/api/claude/question", {
    method: "POST",
    body: JSON.stringify({ requestId, answers }),
  });

export const submitPlan = (requestId: string, note: string | null) =>
  request<{ ok: boolean }>("/api/claude/plan", {
    method: "POST",
    body: JSON.stringify({ requestId, note }),
  });

export type CodexDecision =
  | "accept"
  | "acceptForSession"
  | "decline"
  | "cancel";

export const submitCodexApproval = (
  requestId: string,
  decision: CodexDecision,
) =>
  request<{ ok: boolean }>("/api/codex/approval", {
    method: "POST",
    body: JSON.stringify({ requestId, decision }),
  });

export interface OpenCodeTarget {
  pluginInstanceId: string;
  sessionId: string;
  reviewId: string;
  requestId?: string;
}

export const submitOpenCodeQuestion = (
  target: OpenCodeTarget,
  answers: string[][],
) =>
  request<{ ok: boolean }>("/api/opencode/question", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId: target.requestId ?? target.reviewId, answers }),
  });

export const rejectOpenCodeQuestion = (target: OpenCodeTarget) =>
  request<{ ok: boolean }>("/api/opencode/question/reject", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId: target.requestId ?? target.reviewId, answers: [] }),
  });

export type OpenCodePermissionDecision = "once" | "always" | "reject";

export const submitOpenCodePermission = (
  target: OpenCodeTarget,
  action: OpenCodePermissionDecision,
) =>
  request<{ ok: boolean }>("/api/opencode/permission", {
    method: "POST",
    body: JSON.stringify({
      ...target,
      requestId: target.requestId ?? target.reviewId,
      action,
      message: null,
    }),
  });

export type OpenCodeGateDecision = "allowOnce" | "allowSession" | "reject";

export const submitOpenCodeGate = (
  target: OpenCodeTarget,
  action: OpenCodeGateDecision,
) =>
  request<{ ok: boolean }>("/api/opencode/gate", {
    method: "POST",
    body: JSON.stringify({ ...target, action }),
  });

export interface PiTarget {
  extensionInstanceId: string;
  sessionId: string;
}

export type PiPermissionDecision = "allowOnce" | "allowSession" | "deny";

export const submitPiPermission = (
  target: PiTarget,
  requestId: string,
  decision: PiPermissionDecision,
) =>
  request<{ ok: boolean }>("/api/pi/permission", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId, decision }),
  });

export const submitPiQuestion = (
  target: PiTarget,
  requestId: string,
  answers: LocalQuestionAnswer[],
) =>
  request<{ ok: boolean }>("/api/pi/question", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId, answers }),
  });

export interface DshTarget {
  bridgeInstanceId: string;
  pluginInstanceId: string;
  sessionId: string;
}

export type DshPermissionDecision = "allowOnce" | "deny";

export const submitDshPermission = (
  target: DshTarget,
  requestId: string,
  decision: DshPermissionDecision,
) =>
  request<{ ok: boolean }>("/api/dsh/permission", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId, decision }),
  });

export const submitDshQuestion = (
  target: DshTarget,
  requestId: string,
  answers: LocalQuestionAnswer[],
) =>
  request<{ ok: boolean }>("/api/dsh/question", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId, answers }),
  });

export const submitDshPlan = (
  target: DshTarget,
  requestId: string,
  approved: boolean,
  feedback: string | null,
) =>
  request<{ ok: boolean }>("/api/dsh/plan", {
    method: "POST",
    body: JSON.stringify({ ...target, requestId, approved, feedback }),
  });
