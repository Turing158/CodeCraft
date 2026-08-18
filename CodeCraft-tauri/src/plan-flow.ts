import type { ClaudePlanRequest } from "./claude-sessions";

export interface PendingPlanResolution {
  request: ClaudePlanRequest | undefined;
  dismissedRequestId: string | undefined;
}

export function resolvePendingPlanRequest(
  requests: Array<ClaudePlanRequest | null>,
  dismissedRequestId: string | undefined,
  locallySubmittedRequestId: string | undefined = undefined,
): PendingPlanResolution {
  const pendingRequests = requests.filter(
    (request): request is ClaudePlanRequest => request !== null,
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

  const isDismissedRequest = (request: ClaudePlanRequest) =>
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
