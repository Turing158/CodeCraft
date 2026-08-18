export interface CardSize {
  width: number;
  height: number;
}

export interface CardSizeTransition {
  from: CardSize;
  to: CardSize;
}

export interface CardSizeAnimatorOptions {
  duration?: number;
  easing?: string;
  epsilon?: number;
}

/**
 * Card-like surfaces in the desktop panel. Keep this list explicit so a
 * button or an icon cannot accidentally start a layout animation merely
 * because its class name contains "card".
 */
export const CARD_SIZE_SELECTOR = [
  ".session-source-card",
  ".session-button",
  ".settings-card",
  ".theme-switch__option",
  ".settings-toggle",
  ".hook-agent-button",
  ".about-capability",
  ".detail-section",
  ".question-block",
  ".question-option",
  ".lan-clients",
  ".lan-advanced",
  ".lan-address",
  ".lan-qr",
  ".sound-pack",
  ".custom-sound",
  ".session-list__empty",
  ".hook-card__empty",
].join(", ");

const DEFAULT_EPSILON = 0.5;
const DEFAULT_DURATION = 220;
const DEFAULT_EASING = "cubic-bezier(0.22, 1, 0.36, 1)";

const finitePositive = (value: number) => Number.isFinite(value) && value > 0;

export const cardSizeFor = (element: HTMLElement): CardSize => {
  const rect = element.getBoundingClientRect();
  return { width: rect.width, height: rect.height };
};

export const planCardSizeTransition = (
  from: CardSize,
  to: CardSize,
  epsilon = DEFAULT_EPSILON,
): CardSizeTransition | undefined => {
  if (
    !finitePositive(from.width) ||
    !finitePositive(from.height) ||
    !finitePositive(to.width) ||
    !finitePositive(to.height)
  ) {
    return undefined;
  }

  if (
    Math.abs(from.width - to.width) <= epsilon &&
    Math.abs(from.height - to.height) <= epsilon
  ) {
    return undefined;
  }

  return { from, to };
};

interface ActiveCardAnimation {
  animation: Animation;
  target: CardSize;
  overflow: string;
  willChange: string;
}

/**
 * Animates the used dimensions of card surfaces. CSS transitions cannot
 * interpolate a card whose width is determined by a changing parent, so the
 * observer captures the old and new border-box sizes and animates those
 * measured values with the Web Animations API instead.
 */
export class CardSizeAnimator {
  private readonly duration: number;
  private readonly easing: string;
  private readonly epsilon: number;
  private readonly sizes = new WeakMap<HTMLElement, CardSize>();
  private readonly animations = new Map<HTMLElement, ActiveCardAnimation>();
  private readonly externalAnimations = new Map<HTMLElement, Animation>();
  private readonly resizeObserver: ResizeObserver;
  private readonly mutationObserver: MutationObserver;
  private readonly pending = new Set<HTMLElement>();
  private readonly interruptPending = new WeakSet<HTMLElement>();
  private frame: number | undefined;

  constructor(
    private readonly root: HTMLElement,
    private readonly selector = CARD_SIZE_SELECTOR,
    options: CardSizeAnimatorOptions = {},
  ) {
    this.duration = options.duration ?? DEFAULT_DURATION;
    this.easing = options.easing ?? DEFAULT_EASING;
    this.epsilon = options.epsilon ?? DEFAULT_EPSILON;

    this.resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.target instanceof HTMLElement) {
          this.queue(entry.target, false);
        }
      }
    });
    this.mutationObserver = new MutationObserver((mutations) => {
      for (const mutation of mutations) {
        const target =
          mutation.target instanceof HTMLElement
            ? mutation.target
            : mutation.target.parentElement;
        if (target) this.queueNearestCard(target);

        for (const node of mutation.addedNodes) {
          if (node instanceof HTMLElement) this.observeTree(node);
        }
      }
    });

    this.observeTree(root);
    this.mutationObserver.observe(root, {
      attributes: true,
      attributeFilter: ["hidden", "data-collapsed", "data-expanded"],
      characterData: true,
      childList: true,
      subtree: true,
    });
  }

  /** Re-measure cards after a parent style or CSS variable changes. */
  refresh(): void {
    for (const element of this.root.querySelectorAll<HTMLElement>(this.selector)) {
      this.queue(element, true);
    }
  }

  disconnect(): void {
    this.resizeObserver.disconnect();
    this.mutationObserver.disconnect();
    if (this.frame !== undefined) {
      window.cancelAnimationFrame(this.frame);
      this.frame = undefined;
    }
    this.pending.clear();
    for (const [element, active] of this.animations) {
      this.animations.delete(element);
      active.animation.cancel();
      this.restoreStyles(element, active);
    }
    this.externalAnimations.clear();
  }

  private observeTree(node: HTMLElement): void {
    if (node.matches(this.selector)) this.observeCard(node);
    for (const element of node.querySelectorAll<HTMLElement>(this.selector)) {
      this.observeCard(element);
    }
  }

  private observeCard(element: HTMLElement): void {
    if (!this.sizes.has(element)) {
      const size = cardSizeFor(element);
      if (finitePositive(size.width) && finitePositive(size.height)) {
        this.sizes.set(element, size);
      }
    }
    this.resizeObserver.observe(element);
  }

  private queueNearestCard(element: HTMLElement): void {
    const card = element.matches(this.selector)
      ? element
      : element.closest<HTMLElement>(this.selector);
    if (card) this.queue(card, true);
  }

  private queue(element: HTMLElement, interrupt: boolean): void {
    if (!element.matches(this.selector)) return;
    this.observeCard(element);
    if (interrupt) this.interruptPending.add(element);
    this.pending.add(element);
    if (this.frame !== undefined) return;
    this.frame = window.requestAnimationFrame(() => {
      this.frame = undefined;
      const pending = Array.from(this.pending);
      this.pending.clear();
      for (const card of pending) {
        const interruptAnimation = this.interruptPending.has(card);
        this.interruptPending.delete(card);
        this.measureAndAnimate(card, interruptAnimation);
      }
    });
  }

  private measureAndAnimate(element: HTMLElement, interrupt: boolean): void {
    const external = this.externalSizeAnimation(element);
    if (external) {
      this.waitForExternalAnimation(element, external);
      return;
    }

    const active = this.animations.get(element);
    let from = this.sizes.get(element);
    if (active && !interrupt) return;

    if (active) {
      from = cardSizeFor(element);
      this.animations.delete(element);
      active.animation.cancel();
      this.restoreStyles(element, active);
    }

    const to = cardSizeFor(element);
    if (!from) {
      if (finitePositive(to.width) && finitePositive(to.height)) {
        this.sizes.set(element, to);
      }
      return;
    }

    const transition = planCardSizeTransition(from, to, this.epsilon);
    if (!transition) {
      if (finitePositive(to.width) && finitePositive(to.height)) {
        this.sizes.set(element, to);
      }
      return;
    }

    this.play(element, transition);
  }

  private play(element: HTMLElement, transition: CardSizeTransition): void {
    const previous = this.animations.get(element);
    if (previous) {
      this.animations.delete(element);
      previous.animation.cancel();
      this.restoreStyles(element, previous);
    }

    const active: ActiveCardAnimation = {
      animation: element.animate(
        [
          {
            width: `${transition.from.width}px`,
            height: `${transition.from.height}px`,
          },
          {
            width: `${transition.to.width}px`,
            height: `${transition.to.height}px`,
          },
        ],
        {
          duration: this.duration,
          easing: this.easing,
          fill: "both",
        },
      ),
      target: transition.to,
      overflow: element.style.overflow,
      willChange: element.style.willChange,
    };
    element.style.overflow = "hidden";
    element.style.willChange = "width, height";
    this.animations.set(element, active);
    this.sizes.set(element, transition.to);

    const settle = () => {
      if (this.animations.get(element) !== active) return;
      this.animations.delete(element);
      active.animation.cancel();
      this.restoreStyles(element, active);

      const actual = cardSizeFor(element);
      if (
        planCardSizeTransition(active.target, actual, this.epsilon) !==
        undefined
      ) {
        this.sizes.set(element, active.target);
        this.play(element, {
          from: active.target,
          to: actual,
        });
      } else if (finitePositive(actual.width) && finitePositive(actual.height)) {
        this.sizes.set(element, actual);
      }
    };
    void active.animation.finished.then(settle, settle);
  }

  private externalSizeAnimation(element: HTMLElement): Animation | undefined {
    const animations = element.getAnimations();
    for (const animation of animations) {
      if (this.animations.get(element)?.animation === animation) continue;
      if (!(animation.effect instanceof KeyframeEffect)) continue;
      if (
        animation.effect
          .getKeyframes()
          .some((frame) => frame.width !== undefined || frame.height !== undefined)
      ) {
        return animation;
      }
    }
    return undefined;
  }

  private waitForExternalAnimation(
    element: HTMLElement,
    animation: Animation,
  ): void {
    if (this.externalAnimations.get(element) === animation) return;
    this.externalAnimations.set(element, animation);
    const settle = () => {
      if (this.externalAnimations.get(element) !== animation) return;
      this.externalAnimations.delete(element);
      const size = cardSizeFor(element);
      if (finitePositive(size.width) && finitePositive(size.height)) {
        this.sizes.set(element, size);
      }
    };
    void animation.finished.then(settle, settle);
  }

  private restoreStyles(element: HTMLElement, active: ActiveCardAnimation): void {
    element.style.overflow = active.overflow;
    element.style.willChange = active.willChange;
  }
}
