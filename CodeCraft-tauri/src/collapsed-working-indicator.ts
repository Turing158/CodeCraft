export type WorkingStateRenderer = (working: boolean) => void;

export class CollapsedWorkingIndicator {
  private stopTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly render: WorkingStateRenderer) {}

  start(durationMs?: number): void {
    this.clearTimer();

    if (durationMs !== undefined && durationMs <= 0) {
      this.render(false);
      return;
    }

    this.render(true);

    if (durationMs === undefined || !Number.isFinite(durationMs)) return;

    this.stopTimer = setTimeout(() => {
      this.stopTimer = undefined;
      this.render(false);
    }, durationMs);
  }

  stop(): void {
    this.clearTimer();
    this.render(false);
  }

  dispose(): void {
    this.stop();
  }

  private clearTimer(): void {
    if (this.stopTimer === undefined) return;

    clearTimeout(this.stopTimer);
    this.stopTimer = undefined;
  }
}
