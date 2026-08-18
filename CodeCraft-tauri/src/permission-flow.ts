import type { ClaudePermissionRequest } from "./claude-sessions";

export interface PendingPermissionResolution {
  request: ClaudePermissionRequest | undefined;
  dismissedRequestId: string | undefined;
}

export function resolvePendingPermissionRequest(
  requests: Array<ClaudePermissionRequest | null>,
  dismissedRequestId: string | undefined,
  locallySubmittedRequestId: string | undefined = undefined,
): PendingPermissionResolution {
  const pendingRequests = requests.filter(
    (request): request is ClaudePermissionRequest => request !== null,
  );

  if (!dismissedRequestId) {
    const request = pendingRequests.find(
      (candidate) => candidate.id !== locallySubmittedRequestId,
    );
    return {
      request,
      dismissedRequestId: undefined,
    };
  }

  const isDismissedRequest = (request: ClaudePermissionRequest) =>
    request.id === dismissedRequestId;

  const replacementRequest = pendingRequests.find(
    (request) => !isDismissedRequest(request),
  );
  if (replacementRequest) {
    return {
      request: replacementRequest,
      dismissedRequestId: undefined,
    };
  }

  if (pendingRequests.some(isDismissedRequest)) {
    return {
      request: undefined,
      dismissedRequestId,
    };
  }

  return {
    request: undefined,
    dismissedRequestId: undefined,
  };
}
