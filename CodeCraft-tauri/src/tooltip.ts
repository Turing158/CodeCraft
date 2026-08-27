/**
 * A small, framework-free hover tooltip system.
 *
 * Unlike the native browser `title` tooltip, this implementation:
 *   - draws a themed floating bubble with rounded corners,
 *   - fades in / out with a transparent opacity animation,
 *   - follows the cursor while the pointer stays inside the source element,
 *     and never closes just because the cursor moved within that element,
 *   - closes only once the pointer actually leaves the source element.
 *
 * The tooltip is bound with `pointer-events: none`, so it can never capture
 * the cursor and interrupt a source hover.
 *
 * Tooltip text is resolved from an explicit `data-tooltip` attribute first and
 * falls back to the native `title` attribute. When a native `title` drives the
 * tooltip, it is migrated to `data-tooltip` for the duration of the hover so
 * the browser's own tooltip never renders, and is restored on leave so the
 * attribute stays available (for accessible names and later JS reads).
 */

export const TOOLTIP_ATTR = "data-tooltip";
export const NATIVE_TITLE_ATTR = "title";

export const TOOLTIP_STATE_OPEN = "open";
export const TOOLTIP_STATE_CLOSING = "closing";
export const TOOLTIP_STATE_CLOSED = "closed";

export interface TooltipControllerOptions {
  /** Delay before a freshly hovered source reveals its tooltip. */
  showDelayMs?: number;
  /** Fade-out duration once the pointer leaves the source element. */
  hideDelayMs?: number;
  /** Horizontal offset of the tooltip from the cursor. */
  offsetX?: number;
  /** Vertical offset of the tooltip from the cursor. */
  offsetY?: number;
  /** How many ancestor levels are searched for a tooltip source. */
  maxTraverseDepth?: number;
  /** Minimum distance kept between the tooltip and the viewport edge. */
  viewportGutter?: number;
}

const DEFAULT_SHOW_DELAY_MS = 70;
const DEFAULT_HIDE_DELAY_MS = 120;
const DEFAULT_OFFSET_X = 12;
const DEFAULT_OFFSET_Y = 14;
const DEFAULT_MAX_TRAVERSE_DEPTH = 8;
const DEFAULT_VIEWPORT_GUTTER = 10;

/**
 * Resolve the visible tooltip text from an explicit `data-tooltip` value and a
 * native `title` value. The explicit value wins when present; empty or
 * whitespace-only values resolve to null.
 */
export function resolveTooltipText(
  explicit: string | null,
  nativeTitle: string | null,
): string | null {
  if (explicit !== null) {
    const trimmed = explicit.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
  if (nativeTitle !== null) {
    const trimmed = nativeTitle.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
  return null;
}

/** Resolve the visible tooltip text for an element, or null when absent. */
export function tooltipTextFor(element: HTMLElement): string | null {
  return resolveTooltipText(
    element.getAttribute(TOOLTIP_ATTR),
    element.getAttribute(NATIVE_TITLE_ATTR),
  );
}

/**
 * Attach an explicit custom tooltip to an element. Pass a non-empty string to
 * show a tooltip, or null / empty to remove it. Native `title` remains the
 * fallback when no `data-tooltip` is present.
 */
export function setTooltip(element: HTMLElement, text: string | null): void {
  if (text === null || text.trim().length === 0) {
    element.removeAttribute(TOOLTIP_ATTR);
    return;
  }
  element.setAttribute(TOOLTIP_ATTR, text);
}

export class TooltipController {
  readonly element: HTMLDivElement;

  private readonly showDelayMs: number;
  private readonly hideDelayMs: number;
  private readonly offsetX: number;
  private readonly offsetY: number;
  private readonly maxTraverseDepth: number;
  private readonly viewportGutter: number;

  private activeSource: HTMLElement | undefined;
  private activeText: string | undefined;
  private lastClientX = 0;
  private lastClientY = 0;

  private showTimer: ReturnType<typeof setTimeout> | undefined;
  private hideTimer: ReturnType<typeof setTimeout> | undefined;
  private migratedSources: Array<{
    element: HTMLElement;
    nativeTitle: string;
    borrowed: boolean;
  }> = [];
  private destroyed = false;

  constructor(options: TooltipControllerOptions = {}) {
    this.showDelayMs = options.showDelayMs ?? DEFAULT_SHOW_DELAY_MS;
    this.hideDelayMs = options.hideDelayMs ?? DEFAULT_HIDE_DELAY_MS;
    this.offsetX = options.offsetX ?? DEFAULT_OFFSET_X;
    this.offsetY = options.offsetY ?? DEFAULT_OFFSET_Y;
    this.maxTraverseDepth = options.maxTraverseDepth ?? DEFAULT_MAX_TRAVERSE_DEPTH;
    this.viewportGutter = options.viewportGutter ?? DEFAULT_VIEWPORT_GUTTER;

    this.element = document.createElement("div");
    this.element.className = "codecraft-tooltip";
    this.element.setAttribute("role", "tooltip");
    this.element.hidden = false;
    document.body.append(this.element);

    document.addEventListener("pointermove", this.handlePointerMove);
    // Pointer movement stops being delivered once the cursor leaves the
    // document, so explicitly close the bubble at the document/window edge.
    document.addEventListener("pointerout", this.handlePointerOut, true);
    document.addEventListener("mouseleave", this.handleDocumentLeave);
    document.addEventListener("pointerdown", this.handlePointerDown, true);
    document.addEventListener("keydown", this.handleKeyDown);
    window.addEventListener("blur", this.handleWindowBlur);
    window.addEventListener("resize", this.handleViewportChange);
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    if (this.showTimer !== undefined) clearTimeout(this.showTimer);
    if (this.hideTimer !== undefined) clearTimeout(this.hideTimer);
    this.showTimer = undefined;
    this.hideTimer = undefined;
    document.removeEventListener("pointermove", this.handlePointerMove);
    document.removeEventListener("pointerout", this.handlePointerOut, true);
    document.removeEventListener("mouseleave", this.handleDocumentLeave);
    document.removeEventListener("pointerdown", this.handlePointerDown, true);
    document.removeEventListener("keydown", this.handleKeyDown);
    window.removeEventListener("blur", this.handleWindowBlur);
    window.removeEventListener("resize", this.handleViewportChange);
    this.activeSource = undefined;
    this.activeText = undefined;
    this.restoreMigrated();
    this.element.remove();
  }

  private readonly handlePointerMove = (event: PointerEvent): void => {
    if (this.destroyed) return;
    this.lastClientX = event.clientX;
    this.lastClientY = event.clientY;
    const source = this.resolveSource(event.target);
    if (!source) {
      this.scheduleHide();
      return;
    }
    const text = tooltipTextFor(source);
    if (!text) {
      this.scheduleHide();
      return;
    }
    // The pointer is on a source element again: cancel any pending fade-out.
    if (this.hideTimer !== undefined) {
      clearTimeout(this.hideTimer);
      this.hideTimer = undefined;
    }
    this.ensureMigrated(source);
    const changed =
      source !== this.activeSource || text !== this.activeText;
    this.activeSource = source;
    this.activeText = text;
    if (changed || this.element.dataset.state === TOOLTIP_STATE_CLOSING) {
      this.updateContent(text);
    }
    this.position(event.clientX, event.clientY);
  };

  private readonly handlePointerOut = (event: PointerEvent): void => {
    if (this.destroyed) return;
    const relatedTarget = event.relatedTarget;
    if (relatedTarget instanceof Node && document.contains(relatedTarget)) {
      return;
    }
    this.scheduleHide();
  };

  private readonly handleDocumentLeave = (): void => {
    if (this.destroyed) return;
    this.scheduleHide();
  };

  private readonly handlePointerDown = (_event: PointerEvent): void => {
    // Clicking anywhere dismisses the tooltip immediately.
    if (this.destroyed) return;
    this.hideNow();
  };

  private readonly handleKeyDown = (event: KeyboardEvent): void => {
    if (this.destroyed) return;
    if (event.key === "Escape") this.hideNow();
  };

  private readonly handleWindowBlur = (): void => {
    if (this.destroyed) return;
    this.scheduleHide();
  };

  private readonly handleViewportChange = (): void => {
    if (this.destroyed) return;
    if (!this.activeSource) return;
    this.position(this.lastClientX, this.lastClientY);
  };

  private resolveSource(rawTarget: EventTarget | null): HTMLElement | undefined {
    if (!(rawTarget instanceof HTMLElement)) return undefined;
    let current: HTMLElement | null = rawTarget;
    let depth = 0;
    while (current !== null && depth <= this.maxTraverseDepth) {
      if (tooltipTextFor(current) !== null) return current;
      current = current.parentElement;
      depth += 1;
    }
    return undefined;
  }

  /**
   * Suppress a native `title` tooltip for the current hover. When no explicit
   * `data-tooltip` is present the title is borrowed into `data-tooltip` so the
   * custom bubble can render it; both are restored when the hover ends.
   */
  private ensureMigrated(source: HTMLElement): void {
    const nativeTitle = source.getAttribute(NATIVE_TITLE_ATTR);
    if (nativeTitle === null) return;
    source.removeAttribute(NATIVE_TITLE_ATTR);
    if (source.hasAttribute(TOOLTIP_ATTR)) {
      this.migratedSources.push({ element: source, nativeTitle, borrowed: false });
      return;
    }
    source.setAttribute(TOOLTIP_ATTR, nativeTitle);
    this.migratedSources.push({ element: source, nativeTitle, borrowed: true });
  }

  private updateContent(text: string): void {
    this.element.textContent = text;
    // Already visible: swap the label without a re-animation.
    if (this.element.dataset.state === TOOLTIP_STATE_OPEN) return;
    // Re-entered while still fading out: come back without another reveal delay.
    if (this.element.dataset.state === TOOLTIP_STATE_CLOSING) {
      this.element.dataset.state = TOOLTIP_STATE_OPEN;
      return;
    }
    if (this.showTimer !== undefined) return;
    this.element.dataset.state = TOOLTIP_STATE_CLOSED;
    this.showTimer = setTimeout(() => {
      this.showTimer = undefined;
      if (this.destroyed || !this.activeSource) return;
      this.element.dataset.state = TOOLTIP_STATE_OPEN;
    }, this.showDelayMs);
  }

  private position(clientX: number, clientY: number): void {
    if (!this.activeSource) return;
    this.lastClientX = clientX;
    this.lastClientY = clientY;
    const width = this.element.offsetWidth;
    const height = this.element.offsetHeight;
    const gutter = this.viewportGutter;
    const maxLeft = window.innerWidth - width - gutter;
    const left = Math.max(gutter, Math.min(clientX + this.offsetX, maxLeft));
    let top = clientY + this.offsetY;
    if (top + height > window.innerHeight - gutter) {
      top = Math.max(gutter, clientY - height - this.offsetY);
    }
    this.element.style.left = `${left}px`;
    this.element.style.top = `${top}px`;
  }

  private scheduleHide(): void {
    if (this.destroyed) return;
    // If the pointer left during the reveal delay, cancel the pending show.
    if (this.showTimer !== undefined) {
      clearTimeout(this.showTimer);
      this.showTimer = undefined;
    }
    if (this.activeSource === undefined) return;
    if (this.element.dataset.state !== TOOLTIP_STATE_OPEN) {
      this.activeSource = undefined;
      this.activeText = undefined;
      this.restoreMigrated();
      return;
    }
    if (this.hideTimer !== undefined) return;
    this.element.dataset.state = TOOLTIP_STATE_CLOSING;
    this.hideTimer = setTimeout(() => {
      this.hideTimer = undefined;
      if (this.destroyed) return;
      this.activeSource = undefined;
      this.activeText = undefined;
      this.element.dataset.state = TOOLTIP_STATE_CLOSED;
      this.restoreMigrated();
    }, this.hideDelayMs);
  }

  private hideNow(): void {
    if (this.destroyed) return;
    if (this.showTimer !== undefined) {
      clearTimeout(this.showTimer);
      this.showTimer = undefined;
    }
    if (this.hideTimer !== undefined) clearTimeout(this.hideTimer);
    this.hideTimer = undefined;
    this.activeSource = undefined;
    this.activeText = undefined;
    this.element.dataset.state = TOOLTIP_STATE_CLOSING;
    this.restoreMigrated();
  }

  private restoreMigrated(): void {
    if (this.migratedSources.length === 0) return;
    for (const { element, nativeTitle, borrowed } of this.migratedSources) {
      if (!element.isConnected) continue;
      element.setAttribute(NATIVE_TITLE_ATTR, nativeTitle);
      if (borrowed) element.removeAttribute(TOOLTIP_ATTR);
    }
    this.migratedSources = [];
  }
}
