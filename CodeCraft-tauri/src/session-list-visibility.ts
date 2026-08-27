export const filterDismissedSessions = <T>(
  sessions: T[],
  dismissedKeys: Set<string>,
  keyFor: (session: T) => string,
  isRunning: (session: T) => boolean,
): T[] =>
  sessions.filter((session) => {
    const key = keyFor(session);
    if (!dismissedKeys.has(key)) return true;
    if (!isRunning(session)) return false;

    dismissedKeys.delete(key);
    return true;
  });
