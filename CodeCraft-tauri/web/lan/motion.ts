/** Central motion tokens for the LAN console.
 *
 * Durations and easings match the desktop panel so both surfaces feel the same.
 * Per the workspace instructions there is no prefers-reduced-motion branch; the
 * console exposes its own animation switch instead, and turning it off collapses
 * every duration to zero.
 */

export const MOTION_STORAGE_KEY = "codecraft.lan.motion";

export interface MotionSettings {
  enabled: boolean;
  speed: number;
}

export const DEFAULT_MOTION: MotionSettings = { enabled: true, speed: 1 };
export const MIN_MOTION_SPEED = 0.25;
export const MAX_MOTION_SPEED = 2;

export const EASE_ENTER = "cubic-bezier(0.22, 1, 0.36, 1)";
export const EASE_EXIT = "ease-in";

export const DURATIONS = {
  micro: 90,
  enter: 180,
  view: 240,
  sheet: 260,
} as const;

export type DurationName = keyof typeof DURATIONS;

export const STAGGER_STEP_MS = 24;
export const MAX_STAGGER_STEPS = 12;

export const clampSpeed = (value: number): number => {
  if (!Number.isFinite(value)) return DEFAULT_MOTION.speed;
  return Math.min(MAX_MOTION_SPEED, Math.max(MIN_MOTION_SPEED, value));
};

export const normalizeMotion = (value: unknown): MotionSettings => {
  if (typeof value !== "object" || value === null) return { ...DEFAULT_MOTION };
  const record = value as Record<string, unknown>;
  return {
    enabled: record.enabled !== false,
    speed: clampSpeed(
      typeof record.speed === "number" ? record.speed : DEFAULT_MOTION.speed,
    ),
  };
};

/** A disabled switch yields 0 so animations finish instantly rather than being
 * skipped, which keeps the code paths identical. */
export const scaledDuration = (
  settings: MotionSettings,
  name: DurationName,
): number => (settings.enabled ? Math.round(DURATIONS[name] / settings.speed) : 0);

export const staggerDelay = (
  settings: MotionSettings,
  index: number,
): number => {
  if (!settings.enabled) return 0;
  const steps = Math.min(MAX_STAGGER_STEPS, Math.max(0, index));
  return Math.round((steps * STAGGER_STEP_MS) / settings.speed);
};

export const loadMotion = (storage: Pick<Storage, "getItem">): MotionSettings => {
  try {
    return normalizeMotion(JSON.parse(storage.getItem(MOTION_STORAGE_KEY) ?? "null"));
  } catch {
    return { ...DEFAULT_MOTION };
  }
};

export const saveMotion = (
  storage: Pick<Storage, "setItem">,
  settings: MotionSettings,
): void => {
  try {
    storage.setItem(MOTION_STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // The setting still applies to the current page.
  }
};

export interface AnimationStep {
  keyframes: Keyframe[];
  options: KeyframeAnimationOptions;
}

export const enterAnimation = (
  settings: MotionSettings,
  index = 0,
): AnimationStep => ({
  keyframes: [
    { opacity: 0, transform: "translateY(8px)" },
    { opacity: 1, transform: "translateY(0)" },
  ],
  options: {
    duration: scaledDuration(settings, "enter"),
    delay: staggerDelay(settings, index),
    easing: EASE_ENTER,
    fill: "both",
  },
});

export const viewAnimation = (
  settings: MotionSettings,
  direction: "forward" | "backward",
): AnimationStep => {
  const offset = direction === "forward" ? 18 : -18;
  return {
    keyframes: [
      { opacity: 0, transform: "translateX(" + offset + "px)" },
      { opacity: 1, transform: "translateX(0)" },
    ],
    options: {
      duration: scaledDuration(settings, "view"),
      easing: EASE_ENTER,
      fill: "both",
    },
  };
};

export type SlideEdge = "left" | "right" | "top" | "bottom";

export const sheetAnimation = (
  settings: MotionSettings,
  edge: SlideEdge = "bottom",
): AnimationStep => {
  const transform = {
    left: "translateX(-100%)",
    right: "translateX(100%)",
    top: "translateY(-100%)",
    bottom: "translateY(100%)",
  }[edge];

  return {
    keyframes: [
      { opacity: 0, transform },
      { opacity: 1, transform: "translate(0, 0)" },
    ],
    options: {
      duration: scaledDuration(settings, "sheet"),
      easing: EASE_ENTER,
      fill: "both",
    },
  };
};

export const sheetExitAnimation = (
  settings: MotionSettings,
  edge: SlideEdge = "bottom",
): AnimationStep => {
  const transform = {
    left: "translateX(-100%)",
    right: "translateX(100%)",
    top: "translateY(-100%)",
    bottom: "translateY(100%)",
  }[edge];

  return {
    keyframes: [
      { opacity: 1, transform: "translate(0, 0)" },
      { opacity: 0, transform },
    ],
    options: {
      duration: scaledDuration(settings, "sheet"),
      easing: EASE_EXIT,
      fill: "both",
    },
  };
};

/**
 * Updates live labels without moving their layout.
 *
 * These values are refreshed by every SSE snapshot (and freshness refreshes
 * once per second), so a translateY cross-fade would repeatedly move text up
 * and down while the page is otherwise idle.
 */
export const swapText = (
  element: HTMLElement,
  nextText: string,
  _settings: MotionSettings,
): void => {
  if (element.textContent === nextText) return;
  element.textContent = nextText;
};

export const animate = (
  element: HTMLElement,
  step: AnimationStep,
): Animation | undefined => {
  if (typeof element.animate !== "function") return undefined;
  if (step.options.duration === 0 && !step.options.delay) {
    return undefined;
  }
  return element.animate(step.keyframes, step.options);
};
