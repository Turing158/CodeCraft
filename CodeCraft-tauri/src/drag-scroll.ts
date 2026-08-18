export const DRAG_SCROLL_ACTIVATION_THRESHOLD_PX = 4;

export interface DragScrollOrigin {
  pointerX: number;
  scrollLeft: number;
  maxScrollLeft: number;
}

const clamp = (value: number, max: number) =>
  Math.min(Math.max(value, 0), Math.max(0, max));

/**
 * Tracks a pointer drag over a horizontally scrollable strip and turns pointer
 * movement into scroll offsets. The controller is DOM free so the gesture rules
 * (activation threshold, clamping, click suppression) stay testable.
 */
export class DragScrollController {
  private origin: DragScrollOrigin | undefined;
  private dragging = false;

  constructor(
    private readonly activationThresholdPx = DRAG_SCROLL_ACTIVATION_THRESHOLD_PX,
  ) {}

  /** Begins tracking a pointer. Returns false when the strip cannot scroll. */
  start(origin: DragScrollOrigin): boolean {
    if (!Number.isFinite(origin.pointerX)) return false;
    if (!Number.isFinite(origin.maxScrollLeft) || origin.maxScrollLeft <= 0) {
      return false;
    }

    this.origin = {
      pointerX: origin.pointerX,
      scrollLeft: clamp(origin.scrollLeft, origin.maxScrollLeft),
      maxScrollLeft: origin.maxScrollLeft,
    };
    this.dragging = false;
    return true;
  }

  isTracking(): boolean {
    return this.origin !== undefined;
  }

  isDragging(): boolean {
    return this.dragging;
  }

  /**
   * Feeds a new pointer position. Returns the next scroll offset once the
   * gesture passed the activation threshold, otherwise undefined.
   */
  move(pointerX: number): number | undefined {
    const origin = this.origin;
    if (!origin || !Number.isFinite(pointerX)) return undefined;

    const delta = pointerX - origin.pointerX;
    if (!this.dragging) {
      if (Math.abs(delta) < this.activationThresholdPx) return undefined;
      this.dragging = true;
    }

    return clamp(origin.scrollLeft - delta, origin.maxScrollLeft);
  }

  /** Ends tracking. Returns true when the pointer actually dragged, so the
   * caller can swallow the trailing click. */
  end(): boolean {
    const dragged = this.dragging;
    this.origin = undefined;
    this.dragging = false;
    return dragged;
  }
}
