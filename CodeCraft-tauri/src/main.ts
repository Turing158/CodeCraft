import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  APPEARANCE_SETTINGS_STORAGE_KEY,
  clampAnimationSpeed,
  clampTransparency,
  effectiveAnimationSpeed,
  effectiveStartupAnimationMode,
  effectiveTheme,
  parseAppearanceSettings,
  transparencyIsActive,
  type AppearanceSettings,
  type MotionMode,
  type StartupAnimationMode,
  type ThemeMode,
} from "./appearance-settings";
import {
  BASE_CONTENT_MAX_HEIGHT,
  BASE_SETTINGS_MAX_HEIGHT,
  BASE_SETTINGS_MIN_HEIGHT,
  INTERFACE_SETTINGS_STORAGE_KEY,
  interfaceScaleFor,
  normalizeInterfaceSettings,
  panelWidthFor,
  parseInterfaceSettings,
  type InterfaceSettings,
  type InterfaceSizingMode,
} from "./interface-settings";
import {
  formatSessionTime,
  questionAnswerOptions,
  sessionEntryView,
  type ClaudeAnswerOption,
  type ClaudePlanRequest,
  type ClaudePermissionRequest,
  type ClaudeQuestion,
  type ClaudeQuestionRequest,
  type ClaudeSession,
  type ClaudeSessionSnapshot,
} from "./claude-sessions";
import { initCodexPanel, refreshCodexPanel } from "./codex-panel";
import {
  codexInteractionFor,
  codexPendingInteraction,
  codexPlanRequest,
  codexQuestionRequest,
  type CodexInteraction,
  type CodexSession,
  type CodexSnapshot,
} from "./codex-sessions";
import {
  openCodePendingReveal,
  openCodePermissionRequest,
  openCodeQuestionRequest,
  openCodeReviewForSession,
  openCodeReviewKey,
  openCodeReviewsForSession,
  openCodeSessionKey,
  type OpenCodeReview,
  type OpenCodeNativePermissionReview,
  type OpenCodeStrictToolGateReview,
  type OpenCodeSession,
  type OpenCodeSnapshot,
} from "./opencode-sessions";
import {
  hasUnifiedWorkingSession,
  isCodexSession,
  isOpenCodeSession,
  primaryUnifiedLiveSession,
  unifiedSessionKey,
  unifiedSessionIsRunning,
  unifiedSessionLiveContent,
  unifiedSessionLiveStatusText,
  unifiedSessionSource,
  unifiedSessionStatusLabel,
  unifiedSessionVisualStatus,
  type UnifiedSession,
} from "./unified-sessions";
import {
  COLLAPSE_EXPAND_SETTINGS_STORAGE_KEY,
  autoCollapseDelayMs,
  clampAutoCollapseDelay,
  parseCollapseExpandSettings,
  type CollapseExpandSettings,
} from "./collapse-expand-settings";
import {
  SESSION_CLEANUP_SETTINGS_STORAGE_KEY,
  filterAutoCleanedSessions,
  normalizeSessionCleanupSettings,
  parseSessionCleanupSettings,
  type SessionCleanupPreset,
  type SessionCleanupSettings,
} from "./session-cleanup-settings";
import { filterDismissedSessions } from "./session-list-visibility";
import { CollapsedWorkingIndicator } from "./collapsed-working-indicator";
import {
  captureReviewOrigin,
  contentTransitionDirection,
  reviewReturnView,
  shouldRestoreReviewOrigin,
  type ContentView,
  type ReviewContentView,
} from "./content-view";
import { DragScrollController } from "./drag-scroll";
import { ContextMenuController, type ContextMenuItem } from "./context-menu";
import { TooltipController } from "./tooltip";
import { PanelController } from "./panel-controller";
import { panelClipPath } from "./panel-shape";
import { CARD_SIZE_SELECTOR, CardSizeAnimator } from "./card-size-animation";
import { resolvePendingPermissionRequest } from "./permission-flow";
import {
  createLocalQuestionSubmission,
  createQuestionDrafts,
  questionRequestContentSignature,
  questionActions,
  resolvePendingQuestionRequest,
  updateQuestionSelection,
  type LocalQuestionSubmission,
  type QuestionDraft,
} from "./question-flow";
import { renderMarkdown, renderPlanMarkdown } from "./plan-markdown";
import { resolvePendingPlanRequest } from "./plan-flow";
import { currentLocale, initializeI18n, translateText } from "./i18n";
import {
  UP_TO_DATE_HOLD_MS,
  resolveUpdateCheck,
  resolvedLifecycle,
  type UpdateLifecycle,
} from "./update-check";
import {
  DEFAULT_SOUND_SETTINGS,
  SOUND_EVENTS,
  SOUND_SETTINGS_STORAGE_KEY,
  parseSoundSettings,
  soundEventsForTransition,
  type SoundEvent,
  type SoundPackId,
  type SoundSessionFrame,
  type SoundSettings,
} from "./sound-settings";
import { createSoundPlayer, type PresetPreviewKind } from "./sound-player";
import {
  APPROVALS_RISK_HINT,
  ENABLE_CONFIRM_HINT,
  LAN_ACKNOWLEDGED_STORAGE_KEY,
  LAN_ADVANCED_LOCK_HINT,
  ROTATE_CONFIRM_HINT,
  defaultLanConfig,
  lanAdvancedLocked,
  lanReadOnlyHint,
  lanStateDescriptor,
  lanStatusFor,
  lastClientLabel,
  maskLanToken,
  needsEnableConfirmation,
  parseLanPort,
  planLanCardHeight,
  type LanCardHeightPlan,
  type LanCardHeightSnapshot,
  type LanServerConfigView,
  type LanStatusView,
} from "./lan-settings";
import {
  WINDOW_POSITION_SETTINGS_STORAGE_KEY,
  clampHorizontalPosition,
  parseWindowPositionSettings,
  type WindowPositionSettings,
} from "./window-position-settings";
import {
  createWorkingSquareImageStore,
  validateWorkingSquareImageFile,
  workingSquareImageIsDecodable,
  workingSquareImageThemeForTarget,
  type WorkingSquareImageTheme,
} from "./working-square-images";

initializeI18n();

const translate = (value: string) => translateText(value, currentLocale());

const rootElement = document.documentElement;
const systemMotionPreference = window.matchMedia(
  "(prefers-reduced-motion: reduce)",
);
const systemThemePreference = window.matchMedia("(prefers-color-scheme: dark)");
const presetWorkingSquareImageUrls: Record<WorkingSquareImageTheme, string> = {
  light: new URL(
    "../src-tauri/icons/icon/grass-block.svg?no-inline",
    import.meta.url,
  ).href,
  dark: new URL(
    "../src-tauri/icons/icon/sea-lantern.svg?no-inline",
    import.meta.url,
  ).href,
};
const workingSquareImageStore = createWorkingSquareImageStore();
const cssImageUrl = (url: string) =>
  `url("${url.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}")`;

const loadAppearanceSettings = () => {
  try {
    return parseAppearanceSettings(
      window.localStorage.getItem(APPEARANCE_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return parseAppearanceSettings(null);
  }
};

const loadInterfaceSettings = () => {
  try {
    return parseInterfaceSettings(
      window.localStorage.getItem(INTERFACE_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return parseInterfaceSettings(null);
  }
};

const loadWindowPositionSettings = () => {
  try {
    return parseWindowPositionSettings(
      window.localStorage.getItem(WINDOW_POSITION_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return parseWindowPositionSettings(null);
  }
};

const loadSessionCleanupSettings = () => {
  try {
    return parseSessionCleanupSettings(
      window.localStorage.getItem(SESSION_CLEANUP_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return parseSessionCleanupSettings(null);
  }
};

let appearanceSettings = loadAppearanceSettings();
let interfaceSettings = loadInterfaceSettings();
let windowPositionSettings = loadWindowPositionSettings();
let sessionCleanupSettings = loadSessionCleanupSettings();

const workingSquareImageUrlFor = (theme: WorkingSquareImageTheme) =>
  workingSquareImageStore.urlFor(theme) ?? presetWorkingSquareImageUrls[theme];

const workingSquareImageFileFor = (theme: WorkingSquareImageTheme) =>
  theme === "light"
    ? appearanceSettings.workingSquareLightImageFile
    : appearanceSettings.workingSquareDarkImageFile;

const currentInterfaceScale = () => interfaceScaleFor(interfaceSettings);
const currentPanelWidth = () => panelWidthFor(interfaceSettings);
const currentNativePanelWidth = () =>
  currentPanelWidth() * currentInterfaceScale();

const applyInterfaceSettings = () => {
  const proportional = interfaceSettings.mode === "scale";
  const scale = currentInterfaceScale();
  const fontScale = proportional ? 1 : interfaceSettings.customFontScale;
  const padding = proportional ? 18 : interfaceSettings.customPadding;
  const minimumHeight = proportional
    ? BASE_SETTINGS_MIN_HEIGHT
    : interfaceSettings.customMinHeight;
  const maximumHeight = proportional
    ? BASE_SETTINGS_MAX_HEIGHT
    : interfaceSettings.customMaxHeight;

  rootElement.dataset.interfaceSizing = interfaceSettings.mode;
  rootElement.style.setProperty("--interface-scale", String(scale));
  rootElement.style.setProperty("--font-scale", String(fontScale));
  rootElement.style.setProperty("--panel-width", `${currentPanelWidth()}px`);
  rootElement.style.setProperty("--panel-inline-padding", `${padding}px`);
  rootElement.style.setProperty("--settings-min-height", `${minimumHeight}px`);
  rootElement.style.setProperty("--settings-max-height", `${maximumHeight}px`);
};

const currentAnimationSpeed = () =>
  effectiveAnimationSpeed(appearanceSettings, systemMotionPreference.matches);

const applyMotionToAnimation = (animation: Animation) => {
  const speed = currentAnimationSpeed();

  try {
    if (speed > 0) {
      animation.playbackRate = speed;
      return;
    }

    animation.playbackRate = 1;
    const endTime = animation.effect?.getComputedTiming().endTime;
    if (typeof endTime === "number" && Number.isFinite(endTime)) {
      animation.finish();
    } else {
      animation.cancel();
    }
  } catch {
    animation.cancel();
  }
};

const nativeElementAnimate = Element.prototype.animate;
Element.prototype.animate = function (
  keyframes: Keyframe[] | PropertyIndexedKeyframes | null,
  options?: number | KeyframeAnimationOptions,
) {
  const animation = nativeElementAnimate.call(this, keyframes, options);
  applyMotionToAnimation(animation);
  return animation;
};

const syncRunningAnimations = () => {
  for (const animation of document.getAnimations()) {
    applyMotionToAnimation(animation);
  }
};

const applyAppearanceSettings = (syncAnimations = true) => {
  const animationSpeed = currentAnimationSpeed();
  const transparencyActive = transparencyIsActive(
    appearanceSettings.transparency,
  );
  const resolvedTheme = effectiveTheme(
    appearanceSettings.theme,
    systemThemePreference.matches,
  );
  rootElement.dataset.theme = resolvedTheme;
  rootElement.dataset.workingSquareLightSvg =
    appearanceSettings.workingSquareLightSvg;
  rootElement.dataset.workingSquareDarkSvg =
    appearanceSettings.workingSquareDarkSvg;
  rootElement.dataset.motion = animationSpeed > 0 ? "on" : "off";
  rootElement.dataset.transparency = transparencyActive ? "on" : "off";
  rootElement.style.setProperty(
    "--panel-opacity",
    String(appearanceSettings.transparency),
  );
  rootElement.style.setProperty(
    "--surface-opacity",
    String(appearanceSettings.cardTransparency),
  );
  rootElement.style.setProperty(
    "--surface-hover-opacity",
    String(appearanceSettings.cardTransparency),
  );
  rootElement.style.setProperty(
    "--text-opacity",
    `${Number((appearanceSettings.textTransparency * 100).toFixed(2))}%`,
  );
  rootElement.style.setProperty(
    "--active-animation-speed",
    String(animationSpeed || 1),
  );
  rootElement.style.setProperty(
    "--working-square-image",
    cssImageUrl(workingSquareImageUrlFor(resolvedTheme)),
  );
  if (syncAnimations) syncRunningAnimations();
};

const persistAppearanceSettings = () => {
  try {
    window.localStorage.setItem(
      APPEARANCE_SETTINGS_STORAGE_KEY,
      JSON.stringify(appearanceSettings),
    );
  } catch {
    // Appearance still applies for the current window if persistence is unavailable.
  }
};

const handleAnimationStart = () => {
  window.requestAnimationFrame(syncRunningAnimations);
};

document.addEventListener("animationstart", handleAnimationStart, true);
document.addEventListener("transitionrun", handleAnimationStart, true);
applyAppearanceSettings(false);
applyInterfaceSettings();

const codeCraftIconUrl = new URL("../src-tauri/icons/ico.svg", import.meta.url)
  .href;
// Keep the small logo external because Tauri's CSP rejects Vite data URLs.
const claudeCodeIconUrl = new URL(
  "../src-tauri/icons/icon/claude.svg?no-inline",
  import.meta.url,
).href;
const codexIconUrl = new URL(
  "../src-tauri/icons/icon/openai.svg",
  import.meta.url,
).href;
const openCodeIconUrl = new URL(
  "../src-tauri/icons/icon/opencode.svg?no-inline",
  import.meta.url,
).href;
// The pack artwork stays external for the same CSP reason as the logo above:
// these files are small enough that Vite would otherwise inline them.
const soundPackIconUrls: Record<Exclude<SoundPackId, "custom">, string> = {
  "note-block": new URL(
    "../src-tauri/icons/icon/note_block.svg?no-inline",
    import.meta.url,
  ).href,
  cat: new URL("../src-tauri/icons/icon/cat.svg?no-inline", import.meta.url)
    .href,
  "bone-block": new URL(
    "../src-tauri/icons/icon/bone_block.svg?no-inline",
    import.meta.url,
  ).href,
  experience: new URL(
    "../src-tauri/icons/icon/experience.svg?no-inline",
    import.meta.url,
  ).href,
};

const panel = document.querySelector<HTMLElement>("#panel");
const panelBody = panel?.querySelector<HTMLElement>(".panel__body");
const panelDragRegion = panel?.querySelector<HTMLElement>("#panel-drag-region");
const collapsedWorkingLeadSquare = panel?.querySelector<HTMLElement>(
  ".collapsed-working-flow__square:first-child",
);
const startupSequence = panel?.querySelector<HTMLElement>("#startup-sequence");
const startupAppIcon =
  panel?.querySelector<HTMLImageElement>("#startup-app-icon");
const panelViewStage = panel?.querySelector<HTMLElement>(".panel-view-stage");
const sessionLiveView =
  panel?.querySelector<HTMLButtonElement>("#session-live-view");
const sessionLiveCollapse = panel?.querySelector<HTMLButtonElement>(
  "#session-live-collapse",
);
const sessionLiveStatus = panel?.querySelector<HTMLElement>(
  "#session-live-status",
);
const sessionLiveTitle = panel?.querySelector<HTMLElement>(
  "#session-live-title",
);
const sessionLiveStatusTextElement = panel?.querySelector<HTMLElement>(
  "#session-live-status-text",
);
const collapseHandle =
  panel?.querySelector<HTMLButtonElement>("#collapse-handle");
const integrationState =
  panel?.querySelector<HTMLElement>("#integration-state");
const sessionSummary = panel?.querySelector<HTMLElement>("#session-summary");
const sessionList = panel?.querySelector<HTMLUListElement>(
  "#claude-session-list",
);
const sessionSourceCards = panel?.querySelector<HTMLElement>(
  "#session-source-cards",
);
const claudeSessionCard = panel?.querySelector<HTMLElement>(
  "#claude-session-card",
);
const codexSessionCard = panel?.querySelector<HTMLElement>(
  "#codex-session-card",
);
const openCodeSessionCard = panel?.querySelector<HTMLElement>(
  "#opencode-session-card",
);
const codexSessionList = panel?.querySelector<HTMLUListElement>(
  "#codex-session-list",
);
const openCodeSessionList = panel?.querySelector<HTMLUListElement>(
  "#opencode-session-list",
);
const claudeConnectionStatus = panel?.querySelector<HTMLButtonElement>(
  "#claude-connection-status",
);
const openCodeConnectionStatus = panel?.querySelector<HTMLElement>(
  "#opencode-connection-status",
);
const claudeSessionCardIcon = panel?.querySelector<HTMLImageElement>(
  "#claude-session-card-icon",
);
const codexSessionCardIcon = panel?.querySelector<HTMLImageElement>(
  "#codex-session-card-icon",
);
const openCodeSessionCardIcon = panel?.querySelector<HTMLImageElement>(
  "#opencode-session-card-icon",
);
const sessionProduct = panel?.querySelector<HTMLElement>(".session-product");
const sessionProductTrigger = panel?.querySelector<HTMLButtonElement>(
  "#session-product-trigger",
);
const sessionProductIcon = panel?.querySelector<HTMLElement>(
  "#session-product-icon",
);
const sessionProductTitle = panel?.querySelector<HTMLElement>("#session-title");
const sessionProductMenu = panel?.querySelector<HTMLElement>(
  "#session-product-menu",
);
const settingsOpenButton =
  panel?.querySelector<HTMLButtonElement>("#settings-open");
const sessionView = panel?.querySelector<HTMLElement>("#session-view");
const sessionDetailView = panel?.querySelector<HTMLElement>(
  "#session-detail-view",
);
const sessionDetailBack = panel?.querySelector<HTMLButtonElement>(
  "#session-detail-back",
);
const sessionDetailTitle = panel?.querySelector<HTMLElement>(
  "#session-detail-title",
);
const sessionDetailStatus = panel?.querySelector<HTMLElement>(
  "#session-detail-status",
);
const activitySummary = panel?.querySelector<HTMLElement>("#activity-summary");
const sessionActivityList = panel?.querySelector<HTMLOListElement>(
  "#session-activity-list",
);
const sessionOutput = panel?.querySelector<HTMLElement>("#session-output");
const settingsView = panel?.querySelector<HTMLElement>("#settings-view");
const settingsBackButton =
  panel?.querySelector<HTMLButtonElement>("#settings-back");
const settingsNavShell = panel?.querySelector<HTMLElement>(
  "#settings-nav-shell",
);
const settingsNav = panel?.querySelector<HTMLElement>("#settings-nav");
const settingsNavIndicator = panel?.querySelector<HTMLElement>(
  "#settings-nav-indicator",
);
const settingsPanel = panel?.querySelector<HTMLElement>("#settings-panel");
const settingsPlaceholder = panel?.querySelector<HTMLElement>(
  "#settings-placeholder",
);
const settingsPlaceholderText = panel?.querySelector<HTMLElement>(
  "#settings-placeholder-text",
);
const generalSettings = panel?.querySelector<HTMLElement>("#general-settings");
const sessionCleanupSelect = panel?.querySelector<HTMLElement>(
  "#session-cleanup-select",
);
const sessionCleanupTrigger = panel?.querySelector<HTMLButtonElement>(
  "#session-cleanup-trigger",
);
const sessionCleanupLabel = panel?.querySelector<HTMLElement>(
  "#session-cleanup-label",
);
const sessionCleanupMenu = panel?.querySelector<HTMLElement>(
  "#session-cleanup-menu",
);
const sessionCleanupPresetOptions = panel
  ? Array.from(
      panel.querySelectorAll<HTMLButtonElement>("[data-cleanup-preset]"),
    )
  : [];
const sessionCleanupCustom = panel?.querySelector<HTMLElement>(
  "#session-cleanup-custom",
);
const sessionCleanupCustomMinutesInput = panel?.querySelector<HTMLInputElement>(
  "#session-cleanup-custom-minutes",
);
const topDragEnabledInput =
  panel?.querySelector<HTMLInputElement>("#top-drag-enabled");
const windowPositionAdvancedToggle = panel?.querySelector<HTMLButtonElement>(
  "#window-position-advanced-toggle",
);
const windowPositionAdvanced = panel?.querySelector<HTMLElement>(
  "#window-position-advanced",
);
const windowPositionInput = panel?.querySelector<HTMLInputElement>(
  "#window-horizontal-position",
);
const windowPositionOutput = panel?.querySelector<HTMLOutputElement>(
  "#window-horizontal-position-output",
);
const windowPositionPresetButtons = panel
  ? Array.from(
      panel.querySelectorAll<HTMLButtonElement>("[data-position-preset]"),
    )
  : [];
const hookSettings = panel?.querySelector<HTMLElement>("#hook-settings");
const hookRefreshButton =
  panel?.querySelector<HTMLButtonElement>("#hook-refresh");
const hookRefreshLabel = panel?.querySelector<HTMLElement>(
  "#hook-refresh-label",
);
const installedHookCard = panel?.querySelector<HTMLElement>(
  "#installed-hook-card",
);
const availableHookCard = panel?.querySelector<HTMLElement>(
  "#available-hook-card",
);
const installedHookList = panel?.querySelector<HTMLElement>(
  "#installed-hook-list",
);
const availableHookList = panel?.querySelector<HTMLElement>(
  "#available-hook-list",
);
const installedHookEmpty = panel?.querySelector<HTMLElement>(
  "#installed-hook-empty",
);
const availableHookEmpty = panel?.querySelector<HTMLElement>(
  "#available-hook-empty",
);
const installedHookCount = panel?.querySelector<HTMLElement>(
  "#installed-hook-count",
);
const availableHookCount = panel?.querySelector<HTMLElement>(
  "#available-hook-count",
);
const hookSettingsStatus = panel?.querySelector<HTMLElement>(
  "#hook-settings-status",
);
const lanSettings = panel?.querySelector<HTMLElement>("#lan-settings");
const lanState = panel?.querySelector<HTMLElement>("#lan-state");
const lanEnabledInput = panel?.querySelector<HTMLInputElement>("#lan-enabled");
const lanError = panel?.querySelector<HTMLElement>("#lan-error");
const lanAddressCard = panel?.querySelector<HTMLElement>("#lan-address-card");
const lanAddressList = panel?.querySelector<HTMLElement>("#lan-address-list");
const lanAddressEmpty = panel?.querySelector<HTMLElement>("#lan-address-empty");
const lanQr = panel?.querySelector<HTMLElement>("#lan-qr");
const lanAdvancedToggle = panel?.querySelector<HTMLButtonElement>(
  "#lan-advanced-toggle",
);
const lanAdvanced = panel?.querySelector<HTMLElement>("#lan-advanced");
const lanAdvancedLock = panel?.querySelector<HTMLElement>("#lan-advanced-lock");
const lanPortInput = panel?.querySelector<HTMLInputElement>("#lan-port");
const lanPortApplyButton =
  panel?.querySelector<HTMLButtonElement>("#lan-port-apply");
const lanTokenText = panel?.querySelector<HTMLElement>("#lan-token");
const lanTokenRevealButton =
  panel?.querySelector<HTMLButtonElement>("#lan-token-reveal");
const lanTokenCopyButton =
  panel?.querySelector<HTMLButtonElement>("#lan-token-copy");
const lanTokenRotateButton =
  panel?.querySelector<HTMLButtonElement>("#lan-token-rotate");
const lanAllowApprovalsInput = panel?.querySelector<HTMLInputElement>(
  "#lan-allow-approvals",
);
const lanAuditRemoteInput =
  panel?.querySelector<HTMLInputElement>("#lan-audit-remote");
const lanClients = panel?.querySelector<HTMLElement>("#lan-clients");
const lanClientCount = panel?.querySelector<HTMLElement>("#lan-client-count");
const lanLastClient = panel?.querySelector<HTMLElement>("#lan-last-client");
const approvalModeInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>('input[name="approval-mode"]'),
    )
  : [];
const autoCollapseDelayInput = panel?.querySelector<HTMLInputElement>(
  "#auto-collapse-delay",
);
const autoCollapseDelayOutput = panel?.querySelector<HTMLOutputElement>(
  "#auto-collapse-delay-output",
);
const approvalAutoExpandInput = panel?.querySelector<HTMLInputElement>(
  "#approval-auto-expand",
);
const themeSettings = panel?.querySelector<HTMLElement>("#theme-settings");
const interfaceSizingModeInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>(
        'input[name="interface-sizing-mode"]',
      ),
    )
  : [];
const interfaceScaleSettings = panel?.querySelector<HTMLElement>(
  "#interface-scale-settings",
);
const interfaceCustomSettings = panel?.querySelector<HTMLElement>(
  "#interface-custom-settings",
);
const interfaceScaleInput =
  panel?.querySelector<HTMLInputElement>("#interface-scale");
const interfaceScaleOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-scale-output",
);
const interfaceFontScaleInput = panel?.querySelector<HTMLInputElement>(
  "#interface-font-scale",
);
const interfaceFontScaleOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-font-scale-output",
);
const interfaceWidthInput =
  panel?.querySelector<HTMLInputElement>("#interface-width");
const interfaceWidthOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-width-output",
);
const interfaceMinHeightInput = panel?.querySelector<HTMLInputElement>(
  "#interface-min-height",
);
const interfaceMinHeightOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-min-height-output",
);
const interfaceMaxHeightInput = panel?.querySelector<HTMLInputElement>(
  "#interface-max-height",
);
const interfaceMaxHeightOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-max-height-output",
);
const interfacePaddingInput =
  panel?.querySelector<HTMLInputElement>("#interface-padding");
const interfacePaddingOutput = panel?.querySelector<HTMLOutputElement>(
  "#interface-padding-output",
);
const aboutSettings = panel?.querySelector<HTMLElement>("#about-settings");
const aboutAppIcon = panel?.querySelector<HTMLImageElement>("#about-app-icon");
const aboutVersion = panel?.querySelector<HTMLElement>("#about-version");
const aboutDetailVersion = panel?.querySelector<HTMLElement>(
  "#about-detail-version",
);
const aboutUpdateCheckButton = panel?.querySelector<HTMLButtonElement>(
  "#about-update-check-button",
);
const aboutUpdateNoneButton = panel?.querySelector<HTMLButtonElement>(
  "#about-update-none-button",
);
const aboutUpdateDownloadButton = panel?.querySelector<HTMLButtonElement>(
  "#about-update-download-button",
);
const aboutUpdateStatusText = panel?.querySelector<HTMLElement>(
  "#about-update-status-text",
);
const aboutUpdateStatusDetail = panel?.querySelector<HTMLElement>(
  "#about-update-status-detail",
);
const aboutUpdateStatusIcon = panel?.querySelector<HTMLElement>(
  "#about-update-status-icon",
);
const aboutUpdateStatusIconSvg = panel?.querySelector<SVGSVGElement>(
  "#about-update-status-icon-svg",
);
const aboutUpdateCheckLabel = panel?.querySelector<HTMLElement>(
  "#about-update-check-label",
);
const aboutUpdateCheckSpinner = panel?.querySelector<HTMLElement>(
  "#about-update-check-spinner",
);
const aboutUpdateDownloadLabel = panel?.querySelector<HTMLElement>(
  "#about-update-download-label",
);
const aboutUpdateDownloadSpinner = panel?.querySelector<HTMLElement>(
  "#about-update-download-spinner",
);
const themeModeInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>('input[name="theme-mode"]'),
    )
  : [];
const workingSquareImageButtons = panel
  ? Array.from(
      panel.querySelectorAll<HTMLButtonElement>(
        "[data-working-square-image-theme]",
      ),
    )
  : [];
const workingSquareImageInput = panel?.querySelector<HTMLInputElement>(
  "#working-square-image-input",
);
const workingSquareImageStatus = panel?.querySelector<HTMLElement>(
  "#working-square-image-status",
);
const motionModeInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>('input[name="motion-mode"]'),
    )
  : [];
const startupAnimationModeFieldset = panel?.querySelector<HTMLFieldSetElement>(
  "#startup-animation-mode",
);
const startupAnimationModeInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>(
        'input[name="startup-animation-mode"]',
      ),
    )
  : [];
const animationSpeedSetting = panel?.querySelector<HTMLElement>(
  "#animation-speed-setting",
);
const animationSpeedInput =
  panel?.querySelector<HTMLInputElement>("#animation-speed");
const animationSpeedValue = panel?.querySelector<HTMLOutputElement>(
  "#animation-speed-value",
);
const soundSettings = panel?.querySelector<HTMLElement>("#sound-settings");
const soundEnabledInput =
  panel?.querySelector<HTMLInputElement>("#sound-enabled");
const soundVolumeInput =
  panel?.querySelector<HTMLInputElement>("#sound-volume");
const soundVolumeOutput = panel?.querySelector<HTMLOutputElement>(
  "#sound-volume-output",
);
const customSoundSettings = panel?.querySelector<HTMLElement>(
  "#custom-sound-settings",
);
const customSoundStatus = panel?.querySelector<HTMLElement>(
  "#custom-sound-status",
);
const soundPackInputs = panel
  ? Array.from(
      panel.querySelectorAll<HTMLInputElement>('input[name="sound-pack"]'),
    )
  : [];
const soundPackPreviewButtons = panel
  ? Array.from(
      panel.querySelectorAll<HTMLButtonElement>("[data-sound-pack-preview]"),
    )
  : [];
const soundPackIconImages = panel
  ? Array.from(
      panel.querySelectorAll<HTMLImageElement>("[data-sound-pack-icon]"),
    )
  : [];
const customSoundInputs = panel
  ? Array.from(panel.querySelectorAll<HTMLInputElement>("[data-custom-sound]"))
  : [];
const customSoundPreviewButtons = panel
  ? Array.from(
      panel.querySelectorAll<HTMLButtonElement>("[data-custom-sound-preview]"),
    )
  : [];
const transparencyInput = panel?.querySelector<HTMLInputElement>(
  "#transparency-value",
);
const transparencyOutput = panel?.querySelector<HTMLOutputElement>(
  "#transparency-output",
);
const cardTransparencyInput = panel?.querySelector<HTMLInputElement>(
  "#card-transparency-value",
);
const cardTransparencyOutput = panel?.querySelector<HTMLOutputElement>(
  "#card-transparency-output",
);
const textTransparencyInput = panel?.querySelector<HTMLInputElement>(
  "#text-transparency-value",
);
const textTransparencyOutput = panel?.querySelector<HTMLOutputElement>(
  "#text-transparency-output",
);
const settingsNavItems = panel
  ? Array.from(panel.querySelectorAll<HTMLButtonElement>(".settings-nav__item"))
  : [];
const questionView = panel?.querySelector<HTMLElement>("#question-view");
const questionProgress = panel?.querySelector<HTMLElement>(
  "#question-block-title",
);
const questionHeader = panel?.querySelector<HTMLElement>("#question-header");
const questionText = panel?.querySelector<HTMLElement>("#question-text");
const questionPreview = panel?.querySelector<HTMLElement>("#question-preview");
const questionFullText = panel?.querySelector<HTMLElement>(
  "#question-full-text",
);
const questionBackButton =
  panel?.querySelector<HTMLButtonElement>("#question-back");
const questionAnswerBlock = panel?.querySelector<HTMLElement>(
  "#question-answer-block",
);
const answerCollapseToggle = panel?.querySelector<HTMLButtonElement>(
  "#answer-collapse-toggle",
);
const answerMode = panel?.querySelector<HTMLElement>("#answer-mode");
const questionOptions = panel?.querySelector<HTMLElement>("#question-options");
const questionExtraBlock = panel?.querySelector<HTMLElement>(
  "#question-extra-block",
);
const questionExtraInput = panel?.querySelector<HTMLTextAreaElement>(
  "#question-extra-input",
);
const questionActionsBlock =
  panel?.querySelector<HTMLElement>("#question-actions");
const questionPreviousButton =
  panel?.querySelector<HTMLButtonElement>("#question-previous");
const questionOpenCodexButton = panel?.querySelector<HTMLButtonElement>(
  "#question-open-codex",
);
const questionRejectButton =
  panel?.querySelector<HTMLButtonElement>("#question-reject");
const questionNextButton =
  panel?.querySelector<HTMLButtonElement>("#question-next");
const questionSubmitButton =
  panel?.querySelector<HTMLButtonElement>("#question-submit");
const questionSubmitStatus = panel?.querySelector<HTMLElement>(
  "#question-submit-status",
);
const permissionView = panel?.querySelector<HTMLElement>("#permission-view");
const permissionBackButton =
  panel?.querySelector<HTMLButtonElement>("#permission-back");
const permissionTool = panel?.querySelector<HTMLElement>("#permission-tool");
const permissionSummaryText = panel?.querySelector<HTMLElement>(
  "#permission-summary-text",
);
const permissionCwd = panel?.querySelector<HTMLElement>("#permission-cwd");
const permissionSubmitStatus = panel?.querySelector<HTMLElement>(
  "#permission-submit-status",
);
const permissionAllowButton =
  panel?.querySelector<HTMLButtonElement>("#permission-allow");
const permissionAlwaysAllowButton = panel?.querySelector<HTMLButtonElement>(
  "#permission-always-allow",
);
const permissionDenyButton =
  panel?.querySelector<HTMLButtonElement>("#permission-deny");
const planView = panel?.querySelector<HTMLElement>("#plan-view");
const planBackButton = panel?.querySelector<HTMLButtonElement>("#plan-back");
const planTool = panel?.querySelector<HTMLElement>("#plan-tool");
const planPreview = panel?.querySelector<HTMLElement>("#plan-preview");
const planSummaryText = panel?.querySelector<HTMLElement>("#plan-summary-text");
const planFullText = panel?.querySelector<HTMLElement>("#plan-full-text");
const planCwd = panel?.querySelector<HTMLElement>("#plan-cwd");
const planSubmitStatus = panel?.querySelector<HTMLElement>(
  "#plan-submit-status",
);
const planAutoButton = panel?.querySelector<HTMLButtonElement>("#plan-auto");
const planAutoRememberButton = panel?.querySelector<HTMLButtonElement>(
  "#plan-auto-remember",
);
const planCustomInput =
  panel?.querySelector<HTMLTextAreaElement>("#plan-custom-input");
const planCustomSubmitButton = panel?.querySelector<HTMLButtonElement>(
  "#plan-custom-submit",
);
const planActionBadge = panel?.querySelector<HTMLElement>("#plan-action-badge");
const planOpenCodexButton =
  panel?.querySelector<HTMLButtonElement>("#plan-open-codex");

const setSourceStatusLabel = (button: HTMLElement, label: string) => {
  const labelElement = button.querySelector<HTMLElement>(
    ".session-source-card__status-label",
  );
  if (labelElement) {
    labelElement.textContent = label;
  } else {
    button.textContent = label;
  }
};

const persistInterfaceSettings = () => {
  try {
    window.localStorage.setItem(
      INTERFACE_SETTINGS_STORAGE_KEY,
      JSON.stringify(interfaceSettings),
    );
  } catch {
    // The current-window layout remains active if persistence is unavailable.
  }
};

const persistWindowPositionSettings = () => {
  try {
    window.localStorage.setItem(
      WINDOW_POSITION_SETTINGS_STORAGE_KEY,
      JSON.stringify(windowPositionSettings),
    );
  } catch {
    // The current-window position remains active if persistence is unavailable.
  }
};

if (
  !panel ||
  !panelBody ||
  !panelDragRegion ||
  !collapsedWorkingLeadSquare ||
  !startupSequence ||
  !startupAppIcon ||
  !panelViewStage ||
  !sessionLiveView ||
  !sessionLiveCollapse ||
  !sessionLiveStatus ||
  !sessionLiveTitle ||
  !sessionLiveStatusTextElement ||
  !collapseHandle ||
  !integrationState ||
  !sessionSummary ||
  !sessionList ||
  !sessionSourceCards ||
  !claudeSessionCard ||
  !codexSessionCard ||
  !openCodeSessionCard ||
  !codexSessionList ||
  !openCodeSessionList ||
  !claudeConnectionStatus ||
  !openCodeConnectionStatus ||
  !claudeSessionCardIcon ||
  !codexSessionCardIcon ||
  !openCodeSessionCardIcon ||
  !sessionProduct ||
  !sessionProductTrigger ||
  !sessionProductIcon ||
  !sessionProductTitle ||
  !sessionProductMenu ||
  !settingsOpenButton ||
  !sessionView ||
  !sessionDetailView ||
  !sessionDetailBack ||
  !sessionDetailTitle ||
  !sessionDetailStatus ||
  !activitySummary ||
  !sessionActivityList ||
  !sessionOutput ||
  !settingsView ||
  !settingsBackButton ||
  !settingsNavShell ||
  !settingsNav ||
  !settingsNavIndicator ||
  !settingsPanel ||
  !settingsPlaceholder ||
  !settingsPlaceholderText ||
  !generalSettings ||
  !sessionCleanupSelect ||
  !sessionCleanupTrigger ||
  !sessionCleanupLabel ||
  !sessionCleanupMenu ||
  sessionCleanupPresetOptions.length !== 5 ||
  !sessionCleanupCustom ||
  !sessionCleanupCustomMinutesInput ||
  !topDragEnabledInput ||
  !windowPositionAdvancedToggle ||
  !windowPositionAdvanced ||
  !windowPositionInput ||
  !windowPositionOutput ||
  windowPositionPresetButtons.length !== 3 ||
  !hookSettings ||
  !hookRefreshButton ||
  !hookRefreshLabel ||
  !installedHookCard ||
  !availableHookCard ||
  !installedHookList ||
  !availableHookList ||
  !installedHookEmpty ||
  !availableHookEmpty ||
  !installedHookCount ||
  !availableHookCount ||
  !hookSettingsStatus ||
  !lanSettings ||
  !lanState ||
  !lanEnabledInput ||
  !lanError ||
  !lanAddressCard ||
  !lanAddressList ||
  !lanAddressEmpty ||
  !lanQr ||
  !lanAdvancedToggle ||
  !lanAdvanced ||
  !lanAdvancedLock ||
  !lanPortInput ||
  !lanPortApplyButton ||
  !lanTokenText ||
  !lanTokenRevealButton ||
  !lanTokenCopyButton ||
  !lanTokenRotateButton ||
  !lanAllowApprovalsInput ||
  !lanAuditRemoteInput ||
  !lanClients ||
  !lanClientCount ||
  !lanLastClient ||
  !autoCollapseDelayInput ||
  !autoCollapseDelayOutput ||
  !approvalAutoExpandInput ||
  !themeSettings ||
  interfaceSizingModeInputs.length !== 2 ||
  !interfaceScaleSettings ||
  !interfaceCustomSettings ||
  !interfaceScaleInput ||
  !interfaceScaleOutput ||
  !interfaceFontScaleInput ||
  !interfaceFontScaleOutput ||
  !interfaceWidthInput ||
  !interfaceWidthOutput ||
  !interfaceMinHeightInput ||
  !interfaceMinHeightOutput ||
  !interfaceMaxHeightInput ||
  !interfaceMaxHeightOutput ||
  !interfacePaddingInput ||
  !interfacePaddingOutput ||
  !aboutSettings ||
  !aboutAppIcon ||
  !aboutVersion ||
  !aboutDetailVersion ||
  !aboutUpdateCheckButton ||
  !aboutUpdateNoneButton ||
  !aboutUpdateDownloadButton ||
  !aboutUpdateCheckLabel ||
  !aboutUpdateCheckSpinner ||
  !aboutUpdateDownloadLabel ||
  !aboutUpdateDownloadSpinner ||
  !aboutUpdateStatusText ||
  !aboutUpdateStatusDetail ||
  !aboutUpdateStatusIcon ||
  !aboutUpdateStatusIconSvg ||
  themeModeInputs.length !== 3 ||
  motionModeInputs.length !== 3 ||
  !startupAnimationModeFieldset ||
  startupAnimationModeInputs.length !== 3 ||
  !animationSpeedSetting ||
  !animationSpeedInput ||
  !animationSpeedValue ||
  !soundSettings ||
  !soundEnabledInput ||
  !soundVolumeInput ||
  !soundVolumeOutput ||
  !customSoundSettings ||
  !customSoundStatus ||
  soundPackInputs.length !== 5 ||
  soundPackPreviewButtons.length !== 8 ||
  soundPackIconImages.length !== 4 ||
  customSoundInputs.length !== 5 ||
  customSoundPreviewButtons.length !== 5 ||
  !transparencyInput ||
  !transparencyOutput ||
  !cardTransparencyInput ||
  !cardTransparencyOutput ||
  !textTransparencyInput ||
  !textTransparencyOutput ||
  settingsNavItems.length === 0 ||
  !questionView ||
  !questionProgress ||
  !questionHeader ||
  !questionText ||
  !questionPreview ||
  !questionFullText ||
  !questionBackButton ||
  !questionAnswerBlock ||
  !answerCollapseToggle ||
  !answerMode ||
  !questionOptions ||
  !questionExtraBlock ||
  !questionExtraInput ||
  !questionActionsBlock ||
  !questionPreviousButton ||
  !questionOpenCodexButton ||
  !questionRejectButton ||
  !questionNextButton ||
  !questionSubmitButton ||
  !questionSubmitStatus ||
  !permissionView ||
  !permissionBackButton ||
  !permissionTool ||
  !permissionSummaryText ||
  !permissionCwd ||
  !permissionSubmitStatus ||
  !permissionAllowButton ||
  !permissionAlwaysAllowButton ||
  !permissionDenyButton ||
  !planView ||
  !planBackButton ||
  !planTool ||
  !planPreview ||
  !planSummaryText ||
  !planFullText ||
  !planCwd ||
  !planSubmitStatus ||
  !planAutoButton ||
  !planAutoRememberButton ||
  !planCustomInput ||
  !planCustomSubmitButton ||
  !planActionBadge ||
  !planOpenCodexButton
) {
  throw new Error("CodeCraft session panel is incomplete");
}

const cardSizeAnimator = new CardSizeAnimator(panel, CARD_SIZE_SELECTOR);

const loadSoundPreference = (): SoundSettings => {
  try {
    return parseSoundSettings(
      window.localStorage.getItem(SOUND_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return {
      ...DEFAULT_SOUND_SETTINGS,
      customFiles: { ...DEFAULT_SOUND_SETTINGS.customFiles },
    };
  }
};

let soundPreference = loadSoundPreference();
const soundPlayer = createSoundPlayer(() => soundPreference);

const persistSoundPreference = () => {
  try {
    window.localStorage.setItem(
      SOUND_SETTINGS_STORAGE_KEY,
      JSON.stringify(soundPreference),
    );
  } catch {
    // The current-window setting still applies when storage is unavailable.
  }
};

let customSoundSettingsVisible = false;
let customSoundVisibilityInitialized = false;

const setCustomSoundSettingsVisible = (visible: boolean) => {
  if (
    customSoundVisibilityInitialized &&
    visible === customSoundSettingsVisible
  ) {
    return;
  }

  customSoundSettingsVisible = visible;
  customSoundVisibilityInitialized = true;
  customSoundSettings.inert = !visible;
  customSoundSettings.setAttribute("aria-hidden", String(!visible));
  // The owning settings card animates its measured size. Keeping the child
  // toggle synchronous avoids two height animations fighting over the same
  // layout.
  customSoundSettings.hidden = !visible;
};

const syncSoundControls = () => {
  soundEnabledInput.checked = soundPreference.enabled;
  soundVolumeInput.value = String(Math.round(soundPreference.volume * 100));
  soundVolumeOutput.textContent = `${Math.round(soundPreference.volume * 100)}%`;
  soundVolumeInput.setAttribute(
    "aria-valuetext",
    `${Math.round(soundPreference.volume * 100)}%`,
  );
  soundVolumeInput.style.setProperty(
    "--range-progress",
    `${Math.round(soundPreference.volume * 100)}%`,
  );
  for (const input of soundPackInputs) {
    input.checked = input.value === soundPreference.pack;
  }
  setCustomSoundSettingsVisible(soundPreference.pack === "custom");
  for (const event of SOUND_EVENTS) {
    const output = customSoundSettings.querySelector<HTMLElement>(
      `#custom-sound-${event}-file`,
    );
    if (!output) continue;
    output.textContent = soundPreference.customFiles[event] ?? "未选择音频";
    output.title = output.textContent;
  }
};

const updateSoundPreference = (changes: Partial<SoundSettings>) => {
  soundPreference = { ...soundPreference, ...changes };
  persistSoundPreference();
  syncSoundControls();
};

const setCustomSoundStatus = (message = "") => {
  customSoundStatus.textContent = message;
};

syncSoundControls();

let activeNativeAnimations = 0;
let panelShapeFrame: number | undefined;
let sessionRefreshTimer: ReturnType<typeof setInterval> | undefined;
let refreshingSessions = false;
let refreshingOpenCodeSessions = false;
let selectedSessionId: string | undefined;
let selectedCodexSessionId: string | undefined;
let selectedOpenCodeSessionKey: string | undefined;
let selectedSessionSource: "claude" | "codex" | "opencode" = "claude";
let lastRefreshError: string | undefined;
let lastOpenCodeRefreshError: string | undefined;
let sessionItems = new Map<string, HTMLLIElement>();
let codexSessionItems = new Map<string, HTMLLIElement>();
let openCodeSessionItems = new Map<string, HTMLLIElement>();
let latestCodexSnapshot: CodexSnapshot = {
  connected: false,
  integrationError: null,
  version: 0,
  sessions: [],
  interactions: [],
};
let dismissedCodexInteractionId: string | undefined;
let lastAutoRevealedCodexInteractionId: string | undefined;
let latestOpenCodeSnapshot: OpenCodeSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
  instances: [],
};
let dismissedOpenCodeReviewId: string | undefined;
let lastAutoRevealedOpenCodeReviewId: string | undefined;
let submittedOpenCodeReviewId: string | undefined;
let followupOpenCodeReviewId: string | undefined;
let activeOpenCodeReview: OpenCodeReview | undefined;
let displayedContentView: ContentView = "sessions";
let requestedContentView: ContentView = "sessions";
let questionOriginView: ContentView | undefined;
let manuallyHiddenQuestionRequestId: string | undefined;
let lastAutoRevealedQuestionRequestId: string | undefined;
let latestSessions: ClaudeSession[] = [];
const dismissedSessionKeys = new Set<string>();
let latestClaudeSnapshot: ClaudeSessionSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
};
const soundFrames = new Map<string, SoundSessionFrame>();
const soundSourcePrimed: Record<"claude" | "codex" | "opencode", boolean> = {
  claude: false,
  codex: false,
  opencode: false,
};

const observeSoundFrames = (
  source: "claude" | "codex" | "opencode",
  frames: Map<string, SoundSessionFrame>,
) => {
  const primed = soundSourcePrimed[source];
  const nextKeys = new Set<string>();
  for (const [sessionId, nextFrame] of frames) {
    const key = `${source}:${sessionId}`;
    nextKeys.add(key);
    if (primed) {
      const previous = soundFrames.get(key);
      for (const event of soundEventsForTransition(previous, nextFrame)) {
        void soundPlayer.playEvent(event);
      }
    }
    soundFrames.set(key, nextFrame);
  }
  for (const key of soundFrames.keys()) {
    if (key.startsWith(`${source}:`) && !nextKeys.has(key)) {
      soundFrames.delete(key);
    }
  }
  soundSourcePrimed[source] = true;
};

const claudeSoundFrame = (session: ClaudeSession): SoundSessionFrame => ({
  status: session.status,
  activityIds: session.activities.map((activity) => activity.id),
  failedActivityIds: session.activities
    .filter((activity) => activity.status === "failed")
    .map((activity) => activity.id),
  permissionId: session.permission?.id ?? null,
  planId: session.plan?.id ?? null,
});

const codexSoundFrame = (
  session: CodexSession,
  interactions: CodexInteraction[],
): SoundSessionFrame => {
  const interaction = codexInteractionFor(session, interactions);
  return {
    status: session.status,
    activityIds: session.activities.map((activity) => activity.id),
    failedActivityIds: session.activities
      .filter((activity) => activity.status === "failed")
      .map((activity) => activity.id),
    permissionId:
      interaction?.kind === "permissionsApproval" && !interaction.resolved
        ? interaction.requestId
        : null,
    planId:
      interaction?.kind === "plan" && !interaction.resolved
        ? interaction.requestId
        : null,
  };
};

const observeClaudeSounds = (sessions: ClaudeSession[]) => {
  observeSoundFrames(
    "claude",
    new Map(sessions.map((session) => [session.id, claudeSoundFrame(session)])),
  );
};

const observeCodexSounds = (
  sessions: CodexSession[],
  interactions: CodexInteraction[],
) => {
  observeSoundFrames(
    "codex",
    new Map(
      sessions.map((session) => [
        session.id,
        codexSoundFrame(session, interactions),
      ]),
    ),
  );
};
const openCodeSoundFrame = (session: OpenCodeSession): SoundSessionFrame => {
  const review = openCodeReviewForSession(session);
  return {
    status: session.status,
    activityIds: session.activities.map((activity) => activity.id),
    failedActivityIds: session.activities
      .filter((activity) => activity.status === "failed")
      .map((activity) => activity.id),
    permissionId:
      review &&
      (review.reviewType === "nativePermission" ||
        review.reviewType === "strictToolGate")
        ? openCodeReviewKey(review)
        : null,
    planId: null,
  };
};

const observeOpenCodeSounds = (sessions: OpenCodeSession[]) => {
  observeSoundFrames(
    "opencode",
    new Map(
      sessions.map((session) => [
        openCodeSessionKey(session),
        openCodeSoundFrame(session),
      ]),
    ),
  );
};
let renderedDetailSignature: string | undefined;
let contentViewTransitionTimer: ReturnType<typeof setTimeout> | undefined;
let incomingViewFrame: number | undefined;
let settingsPanelTransition: Animation | undefined;
let settingsPanelTransitionToken = 0;
let displayedSettingsSection = "general";
let requestedSettingsSection = "general";
type SessionProductId = "all" | "claude-code" | "codex" | "opencode";
type SessionSourceProductId = Exclude<SessionProductId, "all">;
type SessionProduct = {
  id: SessionProductId;
  triggerLabel: string;
  optionLabel: string;
  kind: "codecraft" | "claude" | "codex" | "opencode";
  iconUrl: string;
};

const sessionProducts: SessionProduct[] = [
  {
    id: "all",
    triggerLabel: "CodeCraft",
    optionLabel: "全部",
    kind: "codecraft",
    iconUrl: codeCraftIconUrl,
  },
  {
    id: "claude-code",
    triggerLabel: "Claude Code",
    optionLabel: "Claude Code",
    kind: "claude",
    iconUrl: claudeCodeIconUrl,
  },
  {
    id: "codex",
    triggerLabel: "Codex",
    optionLabel: "Codex",
    kind: "codex",
    iconUrl: codexIconUrl,
  },
  {
    id: "opencode",
    triggerLabel: "OpenCode",
    optionLabel: "OpenCode",
    kind: "opencode",
    iconUrl: openCodeIconUrl,
  },
];
const installedSessionProductIds = new Set<SessionSourceProductId>();
let selectedSessionProductId: SessionProductId = "all";
const SESSION_PRODUCT_MENU_TRANSITION_MS = 180;
const SESSION_PRODUCT_MENU_FADE_MS = 150;
const SESSION_PRODUCT_OPTION_REORDER_MS = 180;
let sessionProductMenuCloseTimer: ReturnType<typeof setTimeout> | undefined;
let sessionProductMenuOpenFrame: number | undefined;
let sessionProductSelectionPending = false;
const SESSION_CLEANUP_MENU_TRANSITION_MS = 180;
const SESSION_CLEANUP_MENU_FADE_MS = 150;
const SESSION_CLEANUP_OPTION_REORDER_MS = 180;
let sessionCleanupMenuCloseTimer: ReturnType<typeof setTimeout> | undefined;
let sessionCleanupMenuOpenFrame: number | undefined;
let sessionCleanupSelectionPending = false;
let questionOptionsMeasureFrame: number | undefined;
let renderedQuestionRequestId: string | undefined;
let activeQuestionRequest: ClaudeQuestionRequest | undefined;
let activeQuestionSource: "claude" | "codex" | "opencode" = "claude";
let activeCodexQuestionThreadId: string | undefined;
let activeQuestionIndex = 0;
let questionDrafts: QuestionDraft[] = [];
let locallySubmittedQuestionRequestId: string | undefined;
const localQuestionSubmissions = new Map<string, LocalQuestionSubmission>();
const questionLayoutAnimations = new Map<HTMLElement, Animation>();
let permissionOriginView: ContentView | undefined;
let manuallyHiddenPermissionRequestId: string | undefined;
let lastAutoRevealedPermissionRequestId: string | undefined;
let renderedPermissionRequestId: string | undefined;
let activePermissionRequest: ClaudePermissionRequest | undefined;
let activePermissionSource: "claude" | "codex" | "opencode" = "claude";
let locallySubmittedPermissionRequestId: string | undefined;
let planOriginView: ContentView | undefined;
let manuallyHiddenPlanRequestId: string | undefined;
let lastAutoRevealedPlanRequestId: string | undefined;
let renderedPlanRequestId: string | undefined;
let activePlanRequest: ClaudePlanRequest | undefined;
let activePlanSource: "claude" | "codex" = "claude";
let activeCodexPlanThreadId: string | undefined;
let locallySubmittedPlanRequestId: string | undefined;
let settingsNavResizeObserver: ResizeObserver | undefined;
let localeLayoutFrame: number | undefined;

interface TextSwapState {
  nextText: string;
  animation: Animation | null;
}

interface RenderedQuestionContent {
  progress: string;
  header: string | null;
  questionText: string;
  answerMode: string;
  options: Map<string, { label: string; description: string | null }>;
  extraVisible: boolean;
  extraText: string;
}

interface ActionButtonVisibilityState {
  targetHidden: boolean;
  animation: Animation | null;
}

interface ActionNavVisibilityState {
  targetHidden: boolean;
  animation: Animation | null;
}

const questionTextSwaps = new Map<HTMLElement, TextSwapState>();
const extraVisibilityAnimations = new Map<HTMLElement, Animation>();
const actionButtonVisibilityStates = new Map<
  HTMLButtonElement,
  ActionButtonVisibilityState
>();
let actionNavVisibilityState: ActionNavVisibilityState | undefined;
let extraHideInProgress = false;
let renderedQuestionContent: RenderedQuestionContent | undefined;
const COLLAPSED_PANEL_HEIGHT = 5;
const LIVE_COLLAPSED_HEIGHT = 48;
const LIVE_COLLAPSED_CORNER_PROGRESS = 0.5;
const STARTUP_PANEL_HEIGHT = 132;
const STARTUP_RAIL_DURATION_MS = 460;
const STARTUP_INTRO_DURATION_MS = 920;
const STARTUP_EXIT_DURATION_MS = 240;
let liveCollapsedHeightActive = false;
const MAX_EXPANDED_PANEL_HEIGHT = BASE_CONTENT_MAX_HEIGHT;
const SETTINGS_MIN_EXPANDED_PANEL_HEIGHT = BASE_SETTINGS_MIN_HEIGHT;
const SETTINGS_MAX_EXPANDED_PANEL_HEIGHT = BASE_SETTINGS_MAX_HEIGHT;
const SESSION_REFRESH_INTERVAL_MS = 600;
const CONTENT_VIEW_TRANSITION_MS = 220;
// Review content grows immediately so the native panel never chases an
// intermediate height. Shrinking keeps a short transition for continuity.
const QUESTION_LAYOUT_TRANSITION_MS = 90;
const QUESTION_LAYOUT_TRANSITION_EASING = "cubic-bezier(0.2, 0.7, 0.35, 0.95)";
const QUESTION_TEXT_FADE_OUT_MS = 110;
const QUESTION_TEXT_FADE_IN_MS = 150;
const QUESTION_OPTION_ENTER_MS = 220;
const QUESTION_EXTRA_TRANSITION_MS = 90;
const QUESTION_ACTION_TRANSITION_MS = 90;
const REOPEN_REQUESTED_EVENT = "reopen-requested";
const isTauriRuntime = "__TAURI_INTERNALS__" in window;

aboutAppIcon.src = codeCraftIconUrl;

for (const image of soundPackIconImages) {
  const pack = image.dataset.soundPackIcon as
    Exclude<SoundPackId, "custom"> | undefined;
  const iconUrl = pack ? soundPackIconUrls[pack] : undefined;
  if (iconUrl) image.src = iconUrl;
}

const syncAboutVersion = async () => {
  if (!isTauriRuntime) return;

  try {
    const version = await getVersion();
    aboutVersion.textContent = version;
    aboutDetailVersion.textContent = version;
    installedCodeVersion = version;
  } catch (error: unknown) {
    console.error("Unable to read the CodeCraft version", error);
  }
  // Whatever the update check resolved with the placeholder version must be
  // re-evaluated now that the installed version is known for certain.
  applyUpdateCheckState();
};

void syncAboutVersion();

const GITHUB_RELEASES_API_URL =
  "https://api.github.com/repos/Turing158/CodeCraft/releases?per_page=1";

/** Latest upstream release tag, or null when the release feed is unreachable. */
let installedCodeVersion = "0.1.1";
let latestReleaseTag: string | null | undefined;
let autoUpdateCheckStarted = false;
let updateLifecycle: UpdateLifecycle = "idle";
let upToDateHoldTimer: number | undefined;

const cancelUpToDateHold = (): void => {
  if (upToDateHoldTimer === undefined) return;
  window.clearTimeout(upToDateHoldTimer);
  upToDateHoldTimer = undefined;
};

const scheduleUpToDateHold = (): void => {
  cancelUpToDateHold();
  upToDateHoldTimer = window.setTimeout(() => {
    upToDateHoldTimer = undefined;
    if (updateLifecycle !== "upToDate") return;
    updateLifecycle = "idle";
    applyUpdateCheckState();
  }, UP_TO_DATE_HOLD_MS);
};

const resolvedState = (): "download" | "upToDate" | "noInfo" =>
  resolveUpdateCheck(installedCodeVersion, latestReleaseTag);

/**
 * Applies the global update state to the About section. Checking and
 * downloading show a loading animation, resolved results stay visible, and
 * "无需更新" reverts back to "检查更新" once its five-minute hold expires.
 */
const applyUpdateCheckState = (): void => {
  const checking = updateLifecycle === "checking";
  const downloading = updateLifecycle === "downloading";

  aboutUpdateCheckButton.hidden =
    updateLifecycle !== "idle" && updateLifecycle !== "checking";
  aboutUpdateCheckButton.disabled = checking;
  aboutUpdateCheckButton.classList.toggle("is-loading", checking);
  aboutUpdateCheckLabel.textContent = checking ? "检测中" : "检查更新";
  aboutUpdateCheckSpinner.hidden = !checking;

  aboutUpdateNoneButton.hidden = updateLifecycle !== "upToDate";
  aboutUpdateNoneButton.disabled = updateLifecycle !== "upToDate";

  aboutUpdateDownloadButton.hidden =
    updateLifecycle !== "download" && updateLifecycle !== "downloading";
  aboutUpdateDownloadButton.disabled = downloading;
  aboutUpdateDownloadButton.classList.toggle("is-loading", downloading);
  aboutUpdateDownloadLabel.textContent = downloading ? "下载中" : "下载更新";
  aboutUpdateDownloadSpinner.hidden = !downloading;

  aboutUpdateStatusIcon.classList.toggle("is-busy", checking || downloading);

  if (checking) {
    aboutUpdateStatusText.textContent = "正在检测更新";
    aboutUpdateStatusDetail.textContent = "正在获取最新版本信息，请稍候…";
  } else if (downloading) {
    aboutUpdateStatusText.textContent = "正在下载更新";
    aboutUpdateStatusDetail.textContent = "正在打开下载页面，请稍候…";
  } else if (updateLifecycle === "download") {
    aboutUpdateStatusText.textContent = "发现新版本";
    aboutUpdateStatusDetail.textContent = "有可用的新版本，可下载更新。";
  } else if (updateLifecycle === "upToDate") {
    aboutUpdateStatusText.textContent = "已是最新版本";
    aboutUpdateStatusDetail.textContent = "当前版本与最新版本一致。";
  } else {
    aboutUpdateStatusText.textContent = "未获取到更新信息";
    aboutUpdateStatusDetail.textContent = "未能获取最新版本信息，可点击检查更新。";
  }
};

const fetchLatestReleaseTag = async (): Promise<void> => {
  try {
    const response = await fetch(GITHUB_RELEASES_API_URL);
    if (!response.ok) throw new Error(`GitHub release API returned ${response.status}`);
    const releases: unknown = await response.json();
    const firstRelease = Array.isArray(releases) ? releases[0] : undefined;
    const tagName =
      firstRelease && typeof firstRelease === "object"
        ? (firstRelease as Record<string, unknown>).tag_name
        : undefined;
    latestReleaseTag =
      typeof tagName === "string" && tagName !== "" ? tagName : undefined;
  } catch (error: unknown) {
    console.error("Unable to check the latest CodeCraft release", error);
    latestReleaseTag = undefined;
  }
};

/** Runs a check once and keeps the resolved state on the About card. */
const checkForUpdate = async (): Promise<void> => {
  if (updateLifecycle === "checking" || updateLifecycle === "downloading")
    return;
  cancelUpToDateHold();
  updateLifecycle = "checking";
  applyUpdateCheckState();
  try {
    await fetchLatestReleaseTag();
  } finally {
    updateLifecycle = resolvedLifecycle(resolvedState());
    applyUpdateCheckState();
    if (updateLifecycle === "upToDate") scheduleUpToDateHold();
  }
};

/** Starts the automatic check only once so the state stays global. */
const ensureAutoUpdateCheck = (): void => {
  if (autoUpdateCheckStarted) return;
  autoUpdateCheckStarted = true;
  void checkForUpdate();
};

aboutUpdateCheckButton.addEventListener("click", () => {
  if (updateLifecycle === "checking" || updateLifecycle === "downloading")
    return;
  void checkForUpdate();
});

aboutUpdateDownloadButton.addEventListener("click", () => {
  if (updateLifecycle === "checking" || updateLifecycle === "downloading")
    return;
  updateLifecycle = "downloading";
  applyUpdateCheckState();
  void invoke("open_release_page")
    .catch((error: unknown) => {
      console.error("Unable to open the release page", error);
    })
    .finally(() => {
      updateLifecycle = resolvedLifecycle(resolvedState());
      applyUpdateCheckState();
    });
});

const measureQuestionViewContentHeight = () => {
  const previousMaxHeight = questionView.style.maxHeight;
  questionView.style.maxHeight = "none";
  const contentHeight = questionView.scrollHeight;
  questionView.style.maxHeight = previousMaxHeight;
  return contentHeight;
};

const measurePanelContentHeight = () => {
  const activeView =
    requestedContentView === "question"
      ? questionView
      : requestedContentView === "detail"
        ? sessionDetailView
        : requestedContentView === "settings"
          ? settingsView
          : requestedContentView === "permission"
            ? permissionView
            : requestedContentView === "plan"
              ? planView
              : sessionView;
  const contentHeight =
    activeView === questionView
      ? measureQuestionViewContentHeight()
      : activeView === sessionView
        ? activeView.offsetHeight
      : activeView.scrollHeight;
  const customSizing = interfaceSettings.mode === "custom";
  const minimumHeight = customSizing
    ? interfaceSettings.customMinHeight
    : activeView === settingsView
      ? SETTINGS_MIN_EXPANDED_PANEL_HEIGHT
      : COLLAPSED_PANEL_HEIGHT;
  const maximumHeight = customSizing
    ? interfaceSettings.customMaxHeight
    : activeView === settingsView
      ? SETTINGS_MAX_EXPANDED_PANEL_HEIGHT
      : MAX_EXPANDED_PANEL_HEIGHT;

  return Math.min(
    maximumHeight,
    Math.max(minimumHeight, Math.ceil(contentHeight)),
  );
};

let expandedPanelHeight = measurePanelContentHeight();

const refreshExpandedPanelHeight = () => {
  const nextHeight = measurePanelContentHeight();
  if (nextHeight === expandedPanelHeight) return false;

  expandedPanelHeight = nextHeight;
  return true;
};

const syncPanelShape = () => {
  const shapeContentHeight = rootElement.hasAttribute("data-startup-phase")
    ? STARTUP_PANEL_HEIGHT
    : expandedPanelHeight;
  // Either value can briefly be stale while the native window resizes: the
  // viewport can retain the old expanded height for short content, while the
  // body can retain its expanded layout after collapse. The smaller height is
  // the visible boundary that the CSS outline must stay inside.
  const measuredPanelHeight = panelBody.offsetHeight;
  const viewportPanelHeight = window.innerHeight / currentInterfaceScale();
  const panelHeight =
    measuredPanelHeight > 0
      ? Math.min(measuredPanelHeight, viewportPanelHeight)
      : viewportPanelHeight;
  panelBody.style.clipPath = panelClipPath(
    panelHeight,
    shapeContentHeight,
    liveCollapsedHeightActive
      ? {
          collapsedHeight: LIVE_COLLAPSED_HEIGHT,
          collapsedCornerProgress: LIVE_COLLAPSED_CORNER_PROGRESS,
          interfaceScale: currentInterfaceScale(),
          panelWidth: currentPanelWidth(),
        }
      : {
          interfaceScale: currentInterfaceScale(),
          panelWidth: currentPanelWidth(),
        },
  );
};

const trackPanelShape = () => {
  panelShapeFrame = undefined;
  syncPanelShape();

  if (activeNativeAnimations > 0) {
    panelShapeFrame = window.requestAnimationFrame(trackPanelShape);
  }
};

const beginPanelShapeTracking = () => {
  activeNativeAnimations += 1;
  syncPanelShape();

  if (panelShapeFrame === undefined) {
    panelShapeFrame = window.requestAnimationFrame(trackPanelShape);
  }
};

const endPanelShapeTracking = () => {
  activeNativeAnimations = Math.max(0, activeNativeAnimations - 1);
  if (activeNativeAnimations > 0) return;

  if (panelShapeFrame !== undefined) {
    window.cancelAnimationFrame(panelShapeFrame);
    panelShapeFrame = undefined;
  }

  syncPanelShape();
};

syncPanelShape();
window.addEventListener("resize", syncPanelShape);

const loadCollapseExpandSettings = () => {
  try {
    return parseCollapseExpandSettings(
      window.localStorage.getItem(COLLAPSE_EXPAND_SETTINGS_STORAGE_KEY),
    );
  } catch {
    return parseCollapseExpandSettings(null);
  }
};

let collapseExpandSettings = loadCollapseExpandSettings();

const controller = new PanelController({
  setNativeExpanded: async (expanded, animateHeight = true) => {
    if (expanded && refreshExpandedPanelHeight()) {
      syncPanelShape();
    }

    if (!isTauriRuntime) return;

    beginPanelShapeTracking();

    try {
      await invoke<void>("set_panel_expanded", {
        expanded,
        panelWidth: currentNativePanelWidth(),
        interfaceScale: currentInterfaceScale(),
        height: expandedPanelHeight * currentInterfaceScale(),
        collapsedHeight: collapsedPanelHeight() * currentInterfaceScale(),
        collapsedCornerProgress: collapsedPanelCornerProgress(),
        animateHeight,
      });
    } finally {
      endPanelShapeTracking();
    }
  },
  setVisualState: (state) => {
    document.documentElement.dataset.panelState = state;
    updateCollapsedLiveView();
  },
  setCollapseSource: (source) => {
    document.documentElement.dataset.collapseSource = source;
    updateCollapsedLiveView();
  },
});
controller.setCollapseDelay(autoCollapseDelayMs(collapseExpandSettings));

const revealPanelForAttention = async () => {
  await controller.pointerEntered();
  if (isTauriRuntime) {
    await invoke("show_panel_for_attention");
  }
};

let collapsedWorkingFlowFinishing = false;

const clearCollapsedWorkingFlow = () => {
  collapsedWorkingFlowFinishing = false;
  document.documentElement.removeAttribute("data-panel-working");
};

const collapsedWorkingFlowIsVisible = () =>
  document.documentElement.dataset.panelState === "collapsed" &&
  !document.documentElement.hasAttribute("data-panel-live");

const finishCollapsedWorkingFlowIfHidden = () => {
  if (!collapsedWorkingFlowFinishing || collapsedWorkingFlowIsVisible()) return;
  clearCollapsedWorkingFlow();
};

const collapsedWorkingIndicator = new CollapsedWorkingIndicator((working) => {
  if (working) {
    collapsedWorkingFlowFinishing = false;
    document.documentElement.setAttribute("data-panel-working", "");
    return;
  }

  if (
    collapsedWorkingFlowFinishing ||
    !document.documentElement.hasAttribute("data-panel-working")
  ) {
    return;
  }

  if (collapsedWorkingFlowIsVisible()) {
    collapsedWorkingFlowFinishing = true;
    return;
  }

  clearCollapsedWorkingFlow();
});

collapsedWorkingLeadSquare.addEventListener("animationiteration", (event) => {
  if (
    !collapsedWorkingFlowFinishing ||
    event.animationName !== "collapsed-working-square-roll"
  ) {
    return;
  }

  clearCollapsedWorkingFlow();
});

export const startCollapsedWorking = (durationMs?: number) => {
  collapsedWorkingIndicator.start(durationMs);
};

export const stopCollapsedWorking = () => {
  collapsedWorkingIndicator.stop();
};

const contentViewElement = (view: ContentView) => {
  const views: Record<ContentView, HTMLElement> = {
    sessions: sessionView,
    detail: sessionDetailView,
    settings: settingsView,
    question: questionView,
    permission: permissionView,
    plan: planView,
  };
  return views[view];
};

const switchContentView = (target: ContentView) => {
  if (target === requestedContentView) return;
  requestedContentView = target;

  if (contentViewTransitionTimer !== undefined) {
    clearTimeout(contentViewTransitionTimer);
    contentViewTransitionTimer = undefined;
  }
  if (incomingViewFrame !== undefined) {
    window.cancelAnimationFrame(incomingViewFrame);
    incomingViewFrame = undefined;
  }

  for (const view of [
    sessionView,
    sessionDetailView,
    settingsView,
    questionView,
    permissionView,
    planView,
  ]) {
    view.classList.remove(
      "panel-view--entering",
      "panel-view--leaving",
      "panel-view--forward",
      "panel-view--backward",
    );
    const isDisplayed = view === contentViewElement(displayedContentView);
    view.hidden = !isDisplayed;
    view.setAttribute("aria-hidden", String(!isDisplayed));
  }

  if (target === displayedContentView) {
    const visibleView = contentViewElement(target);
    for (const view of [
      sessionView,
      sessionDetailView,
      settingsView,
      questionView,
      permissionView,
      planView,
    ]) {
      const visible = view === visibleView;
      view.hidden = !visible;
      view.setAttribute("aria-hidden", String(!visible));
    }
    return;
  }

  const outgoingView = contentViewElement(displayedContentView);
  const incomingView = contentViewElement(target);
  const direction = contentTransitionDirection(displayedContentView, target);
  outgoingView.hidden = false;
  incomingView.hidden = false;
  outgoingView.setAttribute("aria-hidden", "true");
  incomingView.setAttribute("aria-hidden", "false");
  outgoingView.classList.add("panel-view--leaving");
  incomingView.classList.add("panel-view--entering");
  if (direction !== "neutral") {
    outgoingView.classList.add(`panel-view--${direction}`);
    incomingView.classList.add(`panel-view--${direction}`);
  }
  void incomingView.offsetWidth;

  incomingViewFrame = window.requestAnimationFrame(() => {
    incomingViewFrame = undefined;
    incomingView.classList.remove("panel-view--entering");
  });

  contentViewTransitionTimer = setTimeout(() => {
    contentViewTransitionTimer = undefined;
    if (requestedContentView !== target) return;

    outgoingView.hidden = true;
    outgoingView.classList.remove(
      "panel-view--leaving",
      "panel-view--forward",
      "panel-view--backward",
    );
    incomingView.classList.remove(
      "panel-view--forward",
      "panel-view--backward",
    );
    displayedContentView = target;
  }, CONTENT_VIEW_TRANSITION_MS);

  if (target === "question") {
    refreshQuestionPreviewClippedState();
  }
};

const rememberReviewOrigin = (reviewView: ReviewContentView) => {
  if (reviewView === "question") {
    questionOriginView = captureReviewOrigin(
      reviewView,
      requestedContentView,
      questionOriginView,
    );
  } else if (reviewView === "permission") {
    permissionOriginView = captureReviewOrigin(
      reviewView,
      requestedContentView,
      permissionOriginView,
    );
  } else {
    planOriginView = captureReviewOrigin(
      reviewView,
      requestedContentView,
      planOriginView,
    );
  }
};

const hasSelectedSessionDetail = () =>
  selectedSessionSource === "codex"
    ? latestCodexSnapshot.sessions.some(
        (session) => session.id === selectedCodexSessionId,
      )
    : selectedSessionSource === "opencode"
      ? latestOpenCodeSnapshot.sessions.some(
          (session) =>
            openCodeSessionKey(session) === selectedOpenCodeSessionKey,
        )
      : latestSessions.some((session) => session.id === selectedSessionId);

const returnViewForReview = (origin: ContentView | undefined) =>
  reviewReturnView(origin, hasSelectedSessionDetail());

const currentQuestionState = () => {
  const request = activeQuestionRequest;
  const question = request?.questions[activeQuestionIndex];
  const draft = questionDrafts[activeQuestionIndex];
  if (!request || !question || !draft) return undefined;

  return { request, question, draft };
};

type QuestionLayoutSnapshot = Map<HTMLElement, DOMRect>;

const captureQuestionLayout = (): QuestionLayoutSnapshot => {
  if (questionView.hidden) return new Map<HTMLElement, DOMRect>();

  return new Map(
    Array.from(questionView.children)
      .filter(
        (element): element is HTMLElement =>
          element instanceof HTMLElement && !element.hidden,
      )
      .map((element) => [element, element.getBoundingClientRect()]),
  );
};

const animateQuestionLayout = (previousLayout: QuestionLayoutSnapshot) => {
  if (previousLayout.size === 0) return;

  for (const element of Array.from(questionView.children)) {
    if (!(element instanceof HTMLElement) || element.hidden) continue;

    const previousRect = previousLayout.get(element);
    if (!previousRect) continue;

    const nextRect = element.getBoundingClientRect();
    const offsetY = previousRect.top - nextRect.top;
    if (Math.abs(offsetY) < 0.5) continue;
    if (offsetY < 0) continue;

    questionLayoutAnimations.get(element)?.cancel();
    element.style.willChange = "transform";
    const animation = element.animate(
      [
        { transform: `translateY(${offsetY}px)` },
        { transform: "translateY(0)" },
      ],
      {
        duration: QUESTION_LAYOUT_TRANSITION_MS,
        easing: QUESTION_LAYOUT_TRANSITION_EASING,
      },
    );
    questionLayoutAnimations.set(element, animation);

    const clearAnimation = () => {
      if (questionLayoutAnimations.get(element) !== animation) return;

      questionLayoutAnimations.delete(element);
      element.style.removeProperty("will-change");
    };
    animation.addEventListener("finish", clearAnimation, { once: true });
    animation.addEventListener("cancel", clearAnimation, { once: true });
  }
};

const refreshQuestionOptionsExpandedHeight = () => {
  questionOptionsMeasureFrame = undefined;
  const expandedHeight = questionOptions.scrollHeight;
  if (expandedHeight <= 0) return;

  questionOptions.style.setProperty(
    "--question-options-expanded-height",
    `${Math.ceil(expandedHeight)}px`,
  );
};

const scheduleQuestionOptionsMeasurement = () => {
  if (questionOptionsMeasureFrame !== undefined) return;

  if (!questionView.hidden) {
    refreshQuestionOptionsExpandedHeight();
    return;
  }

  questionOptionsMeasureFrame = window.requestAnimationFrame(
    refreshQuestionOptionsExpandedHeight,
  );
};

const refreshQuestionPreviewClippedState = () => {
  const measure = () => {
    const clipped = questionText.scrollHeight > questionText.clientHeight + 1;
    questionPreview.dataset.clipped = String(clipped);
    if (clipped) {
      questionPreview.setAttribute("aria-describedby", "question-popover");
      questionPreview.tabIndex = 0;
    } else {
      questionPreview.removeAttribute("aria-describedby");
      questionPreview.removeAttribute("tabindex");
    }
  };

  if (questionView.hidden) {
    window.requestAnimationFrame(measure);
  } else {
    measure();
  }
};

const refreshPlanPreviewClippedState = () => {
  const measure = () => {
    let clipped =
      planSummaryText.scrollHeight > planSummaryText.clientHeight + 1;
    if (!clipped) {
      const lastChild = planSummaryText.lastElementChild;
      if (lastChild instanceof HTMLElement) {
        const containerBottom = planSummaryText.getBoundingClientRect().bottom;
        const lastChildBottom = lastChild.getBoundingClientRect().bottom;
        clipped = lastChildBottom > containerBottom + 1;
      }
    }
    planPreview.dataset.clipped = String(clipped);
    if (clipped) {
      planPreview.setAttribute("aria-describedby", "plan-popover");
      planPreview.tabIndex = 0;
    } else {
      planPreview.removeAttribute("aria-describedby");
      planPreview.removeAttribute("tabindex");
    }
  };

  if (planView.hidden) {
    window.requestAnimationFrame(measure);
  } else {
    measure();
  }
};

const finishQuestionTextSwap = (element: HTMLElement, state: TextSwapState) => {
  if (questionTextSwaps.get(element) !== state) return;
  questionTextSwaps.delete(element);
  element.style.removeProperty("opacity");
};

const swapQuestionText = (
  element: HTMLElement,
  previousText: string,
  nextText: string,
  animate: boolean,
  setText: (value: string) => void = (value: string) => {
    element.textContent = value;
  },
  onSettled?: () => void,
) => {
  const previous = questionTextSwaps.get(element);
  if (previous) {
    questionTextSwaps.delete(element);
    previous.animation?.cancel();
    setText(previous.nextText);
    element.style.removeProperty("opacity");
  }

  if (!animate || previousText === nextText) {
    setText(nextText);
    element.style.removeProperty("opacity");
    onSettled?.();
    return;
  }

  setText(previousText);
  const state: TextSwapState = { nextText, animation: null };
  const fadeOut = element.animate([{ opacity: 1 }, { opacity: 0 }], {
    duration: QUESTION_TEXT_FADE_OUT_MS,
    easing: QUESTION_LAYOUT_TRANSITION_EASING,
  });
  state.animation = fadeOut;
  questionTextSwaps.set(element, state);

  fadeOut.addEventListener(
    "finish",
    () => {
      if (questionTextSwaps.get(element) !== state) return;
      setText(nextText);
      onSettled?.();
      const fadeIn = element.animate([{ opacity: 0 }, { opacity: 1 }], {
        duration: QUESTION_TEXT_FADE_IN_MS,
        easing: QUESTION_LAYOUT_TRANSITION_EASING,
      });
      state.animation = fadeIn;
      fadeIn.addEventListener(
        "finish",
        () => finishQuestionTextSwap(element, state),
        { once: true },
      );
      fadeIn.addEventListener(
        "cancel",
        () => finishQuestionTextSwap(element, state),
        { once: true },
      );
    },
    { once: true },
  );
  fadeOut.addEventListener(
    "cancel",
    () => finishQuestionTextSwap(element, state),
    { once: true },
  );
};

const cancelExtraVisibilityAnimation = () => {
  const animation = extraVisibilityAnimations.get(questionExtraBlock);
  if (animation) {
    extraVisibilityAnimations.delete(questionExtraBlock);
    animation.cancel();
  }
  extraHideInProgress = false;
  questionExtraBlock.style.removeProperty("opacity");
  questionExtraBlock.style.removeProperty("height");
  questionExtraBlock.style.removeProperty("overflow");
};

const isExtraBlockEffectivelyVisible = () =>
  !questionExtraBlock.hidden && !extraHideInProgress;

const setExtraBlockVisible = (visible: boolean, animate: boolean) => {
  const wasVisible = isExtraBlockEffectivelyVisible();
  if (
    wasVisible === visible &&
    !extraVisibilityAnimations.has(questionExtraBlock)
  ) {
    return;
  }

  cancelExtraVisibilityAnimation();

  if (!animate) {
    questionExtraBlock.hidden = !visible;
    return;
  }

  if (visible) {
    questionExtraBlock.hidden = false;
    const expandedHeight = questionExtraBlock.getBoundingClientRect().height;
    questionExtraBlock.style.overflow = "hidden";
    const animation = questionExtraBlock.animate(
      [
        { height: "0px", opacity: 0 },
        { height: `${expandedHeight}px`, opacity: 1 },
      ],
      {
        duration: QUESTION_EXTRA_TRANSITION_MS,
        easing: QUESTION_LAYOUT_TRANSITION_EASING,
      },
    );
    extraVisibilityAnimations.set(questionExtraBlock, animation);

    const finish = () => {
      if (extraVisibilityAnimations.get(questionExtraBlock) !== animation) {
        return;
      }
      extraVisibilityAnimations.delete(questionExtraBlock);
      questionExtraBlock.style.removeProperty("opacity");
      questionExtraBlock.style.removeProperty("height");
      questionExtraBlock.style.removeProperty("overflow");
    };
    animation.addEventListener("finish", finish, { once: true });
    animation.addEventListener("cancel", finish, { once: true });
    return;
  }

  extraHideInProgress = true;
  const currentHeight = questionExtraBlock.getBoundingClientRect().height;
  const animation = questionExtraBlock.animate(
    [
      { opacity: 1, height: `${currentHeight}px` },
      { opacity: 0, height: "0px" },
    ],
    {
      duration: QUESTION_EXTRA_TRANSITION_MS,
      easing: QUESTION_LAYOUT_TRANSITION_EASING,
    },
  );
  extraVisibilityAnimations.set(questionExtraBlock, animation);

  const finish = () => {
    if (extraVisibilityAnimations.get(questionExtraBlock) !== animation) {
      return;
    }
    extraVisibilityAnimations.delete(questionExtraBlock);
    extraHideInProgress = false;
    questionExtraBlock.hidden = true;
    questionExtraBlock.style.removeProperty("opacity");
    questionExtraBlock.style.removeProperty("height");
    questionExtraBlock.style.removeProperty("overflow");
  };
  animation.addEventListener("finish", finish, { once: true });
  animation.addEventListener("cancel", finish, { once: true });
};

const clearActionButtonTransitionStyles = (button: HTMLButtonElement) => {
  button.style.removeProperty("flex-grow");
  button.style.removeProperty("padding-left");
  button.style.removeProperty("padding-right");
  button.style.removeProperty("opacity");
  button.style.removeProperty("overflow");
  button.style.removeProperty("white-space");
  button.removeAttribute("tabindex");
};

const isActionButtonEffectivelyVisible = (button: HTMLButtonElement) =>
  !button.hidden &&
  actionButtonVisibilityStates.get(button)?.targetHidden !== true;

const setActionButtonVisible = (
  button: HTMLButtonElement,
  visible: boolean,
  animate: boolean,
) => {
  if (isActionButtonEffectivelyVisible(button) === visible) return;

  const previousState = actionButtonVisibilityStates.get(button);
  const enteringFromHidden = visible && button.hidden && !previousState;
  const computedStyle = getComputedStyle(button);
  const startGrow = enteringFromHidden
    ? 0
    : Number(computedStyle.flexGrow || (visible ? 0 : 1));
  const startPadding = enteringFromHidden
    ? 0
    : parseFloat(computedStyle.paddingLeft) || (visible ? 0 : 12);
  const startOpacity = enteringFromHidden
    ? 0
    : Number(computedStyle.opacity || (visible ? 0 : 1));

  if (previousState) {
    actionButtonVisibilityStates.delete(button);
    previousState.animation?.cancel();
  }
  clearActionButtonTransitionStyles(button);

  if (!animate) {
    button.hidden = !visible;
    return;
  }

  if (visible) {
    button.hidden = false;
    const state: ActionButtonVisibilityState = {
      targetHidden: false,
      animation: null,
    };
    button.style.overflow = "hidden";
    button.style.whiteSpace = "nowrap";
    const animation = button.animate(
      [
        {
          flexGrow: String(startGrow),
          paddingLeft: `${startPadding}px`,
          paddingRight: `${startPadding}px`,
          opacity: String(startOpacity),
        },
        {
          flexGrow: "1",
          paddingLeft: "12px",
          paddingRight: "12px",
          opacity: "1",
        },
      ],
      {
        duration: QUESTION_ACTION_TRANSITION_MS,
        easing: QUESTION_LAYOUT_TRANSITION_EASING,
      },
    );
    state.animation = animation;
    actionButtonVisibilityStates.set(button, state);

    const finish = () => {
      if (actionButtonVisibilityStates.get(button) !== state) return;
      actionButtonVisibilityStates.delete(button);
      clearActionButtonTransitionStyles(button);
    };
    animation.addEventListener("finish", finish, { once: true });
    animation.addEventListener("cancel", finish, { once: true });
    return;
  }

  button.hidden = false;
  button.tabIndex = -1;
  const state: ActionButtonVisibilityState = {
    targetHidden: true,
    animation: null,
  };
  button.style.overflow = "hidden";
  button.style.whiteSpace = "nowrap";
  const animation = button.animate(
    [
      {
        flexGrow: String(startGrow),
        paddingLeft: `${startPadding}px`,
        paddingRight: `${startPadding}px`,
        opacity: String(startOpacity),
      },
      {
        flexGrow: "0",
        paddingLeft: "0px",
        paddingRight: "0px",
        opacity: "0",
      },
    ],
    {
      duration: QUESTION_ACTION_TRANSITION_MS,
      easing: QUESTION_LAYOUT_TRANSITION_EASING,
    },
  );
  state.animation = animation;
  actionButtonVisibilityStates.set(button, state);

  const finish = () => {
    if (actionButtonVisibilityStates.get(button) !== state) return;
    actionButtonVisibilityStates.delete(button);
    button.hidden = true;
    clearActionButtonTransitionStyles(button);
  };
  animation.addEventListener("finish", finish, { once: true });
  animation.addEventListener("cancel", finish, { once: true });
};

const clearActionNavTransitionStyles = () => {
  questionActionsBlock.style.removeProperty("height");
  questionActionsBlock.style.removeProperty("opacity");
  questionActionsBlock.style.removeProperty("overflow");
};

const isActionNavEffectivelyVisible = () =>
  !questionActionsBlock.hidden &&
  actionNavVisibilityState?.targetHidden !== true;

const setActionNavVisible = (visible: boolean, animate: boolean) => {
  if (isActionNavEffectivelyVisible() === visible) return;

  const previousState = actionNavVisibilityState;
  if (previousState) {
    actionNavVisibilityState = undefined;
    previousState.animation?.cancel();
  }
  clearActionNavTransitionStyles();

  if (!animate) {
    questionActionsBlock.hidden = !visible;
    return;
  }

  if (visible) {
    questionActionsBlock.hidden = false;
    const state: ActionNavVisibilityState = {
      targetHidden: false,
      animation: null,
    };
    questionActionsBlock.style.opacity = "0";
    const animation = questionActionsBlock.animate(
      [{ opacity: "0" }, { opacity: "1" }],
      {
        duration: QUESTION_ACTION_TRANSITION_MS,
        easing: QUESTION_LAYOUT_TRANSITION_EASING,
      },
    );
    state.animation = animation;
    actionNavVisibilityState = state;

    const finish = () => {
      if (actionNavVisibilityState !== state) return;
      actionNavVisibilityState = undefined;
      clearActionNavTransitionStyles();
    };
    animation.addEventListener("finish", finish, { once: true });
    animation.addEventListener("cancel", finish, { once: true });
    return;
  }

  if (questionActionsBlock.hidden) return;
  const currentHeight = questionActionsBlock.getBoundingClientRect().height;
  const state: ActionNavVisibilityState = {
    targetHidden: true,
    animation: null,
  };
  questionActionsBlock.style.overflow = "hidden";
  const animation = questionActionsBlock.animate(
    [
      { height: `${currentHeight}px`, opacity: "1" },
      { height: "0px", opacity: "0" },
    ],
    {
      duration: QUESTION_ACTION_TRANSITION_MS,
      easing: QUESTION_LAYOUT_TRANSITION_EASING,
    },
  );
  state.animation = animation;
  actionNavVisibilityState = state;

  const finish = () => {
    if (actionNavVisibilityState !== state) return;
    actionNavVisibilityState = undefined;
    questionActionsBlock.hidden = true;
    clearActionNavTransitionStyles();
  };
  animation.addEventListener("finish", finish, { once: true });
  animation.addEventListener("cancel", finish, { once: true });
};

const syncQuestionAnswerState = (
  previousLayout: QuestionLayoutSnapshot | undefined,
  animate = false,
) => {
  const state = currentQuestionState();
  if (!state) return;

  const layout =
    previousLayout ?? (animate ? captureQuestionLayout() : undefined);

  for (const button of questionOptions.querySelectorAll<HTMLButtonElement>(
    ".question-option",
  )) {
    button.setAttribute(
      "aria-pressed",
      String(state.draft.selectedAnswerIds.has(button.dataset.optionId ?? "")),
    );
  }

  const otherSelected = Array.from(state.draft.selectedAnswerIds).some((id) =>
    id.endsWith(":other"),
  );
  setExtraBlockVisible(otherSelected, animate);

  const collapseStateChanged =
    questionAnswerBlock.dataset.collapsed !== String(state.draft.collapsed);
  questionAnswerBlock.dataset.collapsed = String(state.draft.collapsed);
  answerCollapseToggle.setAttribute(
    "aria-expanded",
    String(!state.draft.collapsed),
  );
  const collapseLabel = state.draft.collapsed ? "展开回答选项" : "收起回答选项";
  answerCollapseToggle.ariaLabel = collapseLabel;
  answerCollapseToggle.title = collapseLabel;

  const actions = new Set(
    state.question.readOnly
      ? [
          ...(activeQuestionIndex > 0 ? (["previous"] as const) : []),
          ...(activeQuestionIndex < state.request.questions.length - 1
            ? (["next"] as const)
            : []),
        ]
      : questionActions(
          activeQuestionIndex,
          state.request.questions.length,
          state.draft,
        ),
  );
  const showOpenCodex =
    activeQuestionSource === "codex" && state.question.readOnly === true;
  const showReject = activeQuestionSource === "opencode";
  setActionNavVisible(actions.size > 0 || showOpenCodex || showReject, animate);
  setActionButtonVisible(
    questionPreviousButton,
    actions.has("previous"),
    animate,
  );
  setActionButtonVisible(questionOpenCodexButton, showOpenCodex, animate);
  setActionButtonVisible(questionRejectButton, showReject, animate);
  setActionButtonVisible(questionNextButton, actions.has("next"), animate);
  setActionButtonVisible(questionSubmitButton, actions.has("submit"), animate);
  if (!collapseStateChanged && layout) {
    animateQuestionLayout(layout);
  }
};

const selectQuestionAnswer = (option: ClaudeAnswerOption) => {
  const state = currentQuestionState();
  if (!state || state.question.readOnly) return;

  const wasOtherSelected =
    option.kind === "other" && state.draft.selectedAnswerIds.has(option.id);
  const nextDraft = updateQuestionSelection(
    state.question,
    state.draft,
    option,
  );
  questionDrafts[activeQuestionIndex] = nextDraft;
  syncQuestionAnswerState(undefined, true);

  if (
    option.kind === "other" &&
    !wasOtherSelected &&
    nextDraft.selectedAnswerIds.has(option.id)
  ) {
    window.requestAnimationFrame(() => questionExtraInput.focus());
  }
};

const createQuestionOptionButton = (
  option: ClaudeAnswerOption,
  readOnly: boolean,
): HTMLButtonElement => {
  const button = document.createElement("button");
  button.className = "question-option";
  button.type = "button";
  button.dataset.optionId = option.id;
  button.dataset.optionKind = option.kind;
  button.setAttribute("aria-pressed", "false");
  button.disabled = readOnly;

  const label = document.createElement("span");
  label.className = "question-option__label";
  label.textContent = option.label;
  button.append(label);

  if (option.description) {
    const description = document.createElement("span");
    description.className = "question-option__description";
    description.textContent = option.description;
    button.append(description);
  }

  button.addEventListener("click", () => selectQuestionAnswer(option));
  return button;
};

const renderCurrentQuestion = (animate = false) => {
  const state = currentQuestionState();
  if (!state) return;

  const previousContent = renderedQuestionContent;
  const previousLayout = animate ? captureQuestionLayout() : undefined;
  const { request, question, draft } = state;

  const nextProgress = `第 ${activeQuestionIndex + 1}/${request.questions.length} 问题`;
  const nextHeader = question.header ?? "Claude Code";
  const nextQuestionText = question.question;
  const nextAnswerMode =
    question.answerMode ?? (question.multiSelect ? "可多选" : "单选");
  const nextOptions = questionAnswerOptions(
    request.id,
    question,
    activeQuestionIndex,
  );
  const nextExtraText = draft.extraText;
  const nextExtraVisible = Array.from(draft.selectedAnswerIds).some((id) =>
    id.endsWith(":other"),
  );

  const previousHadHeader = previousContent?.header !== undefined;
  const nextHasHeader = question.header !== undefined;
  questionHeader.hidden = !nextHasHeader;
  if (animate) {
    swapQuestionText(
      questionProgress,
      previousContent?.progress ?? nextProgress,
      nextProgress,
      true,
    );
    if (previousHadHeader && nextHasHeader) {
      swapQuestionText(
        questionHeader,
        previousContent!.header!,
        question.header!,
        true,
      );
    } else {
      questionHeader.textContent = nextHeader;
    }
  } else {
    questionProgress.textContent = nextProgress;
    questionHeader.textContent = nextHeader;
  }

  swapQuestionText(
    questionText,
    previousContent?.questionText ?? nextQuestionText,
    nextQuestionText,
    animate,
    undefined,
    () => refreshQuestionPreviewClippedState(),
  );
  swapQuestionText(
    questionFullText,
    previousContent?.questionText ?? nextQuestionText,
    nextQuestionText,
    animate,
  );
  swapQuestionText(
    answerMode,
    previousContent?.answerMode ?? nextAnswerMode,
    nextAnswerMode,
    animate,
  );

  const previousOptions =
    previousContent?.options ??
    new Map<string, { label: string; description: string | null }>();
  const nextOptionsByLabel = new Map<string, ClaudeAnswerOption>();
  for (const option of nextOptions) {
    nextOptionsByLabel.set(option.label, option);
  }
  questionOptions.replaceChildren(
    ...nextOptions.map((option) =>
      createQuestionOptionButton(option, question.readOnly === true),
    ),
  );

  if (animate) {
    for (const button of questionOptions.querySelectorAll<HTMLButtonElement>(
      ".question-option",
    )) {
      const label = button.querySelector<HTMLElement>(
        ".question-option__label",
      );
      const optionLabel = label?.textContent ?? "";
      const option = nextOptionsByLabel.get(optionLabel);
      if (!option) continue;
      const previous = previousOptions.get(optionLabel);
      const description = button.querySelector<HTMLElement>(
        ".question-option__description",
      );

      if (!previous) {
        button.style.opacity = "0";
        const entrance = button.animate([{ opacity: 0 }, { opacity: 1 }], {
          duration: QUESTION_OPTION_ENTER_MS,
          easing: QUESTION_LAYOUT_TRANSITION_EASING,
        });
        const finish = () => button.style.removeProperty("opacity");
        entrance.addEventListener("finish", finish, { once: true });
        entrance.addEventListener("cancel", finish, { once: true });
        continue;
      }

      if (
        previous.label === option.label &&
        previous.description === option.description
      ) {
        continue;
      }

      if (label) {
        swapQuestionText(label, previous.label, option.label, true);
      }
      if (description) {
        if (previous.description === null) {
          description.style.opacity = "0";
          const entrance = description.animate(
            [{ opacity: 0 }, { opacity: 1 }],
            {
              duration: QUESTION_OPTION_ENTER_MS,
              easing: QUESTION_LAYOUT_TRANSITION_EASING,
            },
          );
          const finish = () => description.style.removeProperty("opacity");
          entrance.addEventListener("finish", finish, { once: true });
          entrance.addEventListener("cancel", finish, { once: true });
        } else if (previous.description !== option.description) {
          swapQuestionText(
            description,
            previous.description,
            option.description ?? "",
            true,
          );
        }
      }
    }
  }

  const previousExtraVisible =
    previousContent?.extraVisible ?? isExtraBlockEffectivelyVisible();
  if (animate && previousExtraVisible && nextExtraVisible) {
    swapQuestionText(
      questionExtraInput,
      previousContent?.extraText ?? nextExtraText,
      nextExtraText,
      true,
      (value: string) => {
        questionExtraInput.value = value;
      },
    );
  } else {
    questionExtraInput.value = nextExtraText;
  }

  scheduleQuestionOptionsMeasurement();
  syncQuestionAnswerState(previousLayout, animate);

  renderedQuestionContent = {
    progress: nextProgress,
    header: question.header,
    questionText: nextQuestionText,
    answerMode: nextAnswerMode,
    options: nextOptionsByLabel,
    extraVisible: nextExtraVisible,
    extraText: nextExtraText,
  };
};

const renderQuestionRequest = (request: ClaudeQuestionRequest) => {
  if (request.id === renderedQuestionRequestId) return;
  if (
    activeQuestionRequest &&
    questionRequestContentSignature(activeQuestionRequest) ===
      questionRequestContentSignature(request)
  ) {
    return;
  }

  const animate = renderedQuestionRequestId !== undefined;
  renderedQuestionRequestId = request.id;
  activeQuestionRequest = request;
  activeQuestionIndex = 0;
  questionDrafts = createQuestionDrafts(request);
  renderCurrentQuestion(animate);
};

const setQuestionSubmitStatus = (
  message: string | undefined,
  state: "pending" | "success" | "error" = "pending",
) => {
  questionSubmitStatus.textContent = message ?? "";
  questionSubmitStatus.hidden = !message;
  if (message) questionSubmitStatus.dataset.state = state;
  else delete questionSubmitStatus.dataset.state;
};

const syncOpenCodeQuestionStatus = (review: OpenCodeReview) => {
  if (review.reviewType !== "question") return;
  if (review.submitting) {
    setQuestionSubmitStatus(translate("正在回传回答…"));
  } else if (review.submissionError) {
    setQuestionSubmitStatus(
      `${translate("回传失败：")}${review.submissionError}`,
      "error",
    );
  } else if (openCodeReviewKey(review) === followupOpenCodeReviewId) {
    setQuestionSubmitStatus(translate("上一条已处理，这是新的问题"), "success");
  } else {
    setQuestionSubmitStatus(undefined);
  }
};

const clearActiveQuestionRequest = () => {
  renderedQuestionRequestId = undefined;
  activeQuestionRequest = undefined;
  activeQuestionIndex = 0;
  questionDrafts = [];
  activeCodexQuestionThreadId = undefined;
  renderedQuestionContent = undefined;
  questionSubmitButton.disabled = false;
  questionRejectButton.disabled = false;
  questionSubmitStatus.hidden = true;
  questionSubmitStatus.textContent = "";
  delete questionSubmitStatus.dataset.state;

  for (const state of questionTextSwaps.values()) {
    state.animation?.cancel();
  }
  questionTextSwaps.clear();
  cancelExtraVisibilityAnimation();
  questionExtraBlock.hidden = true;
  questionExtraInput.value = "";

  for (const [button, state] of actionButtonVisibilityStates) {
    actionButtonVisibilityStates.delete(button);
    state.animation?.cancel();
    clearActionButtonTransitionStyles(button);
    button.hidden = true;
  }
  actionNavVisibilityState?.animation?.cancel();
  actionNavVisibilityState = undefined;
  clearActionNavTransitionStyles();
  questionActionsBlock.hidden = true;
};

const showQuestionAt = (questionIndex: number) => {
  if (!activeQuestionRequest) return;
  if (
    questionIndex < 0 ||
    questionIndex >= activeQuestionRequest.questions.length
  ) {
    return;
  }

  activeQuestionIndex = questionIndex;
  renderCurrentQuestion(true);
};

questionBackButton.addEventListener("click", () => {
  if (!activeQuestionRequest) return;

  if (activeQuestionSource === "codex") {
    dismissedCodexInteractionId = activeQuestionRequest.id;
  } else if (activeQuestionSource === "opencode") {
    dismissedOpenCodeReviewId = activeQuestionRequest.id;
  } else {
    manuallyHiddenQuestionRequestId = activeQuestionRequest.id;
  }
  const returnView = returnViewForReview(questionOriginView);
  questionOriginView = undefined;
  switchContentView(returnView);

  window.requestAnimationFrame(() => {
    const returnFocusTarget =
      returnView === "detail"
        ? sessionDetailBack
        : sessionList.querySelector<HTMLButtonElement>(".session-button");
    returnFocusTarget?.focus({ preventScroll: true });
  });
});

answerCollapseToggle.addEventListener("click", () => {
  const state = currentQuestionState();
  if (!state) return;

  questionDrafts[activeQuestionIndex] = {
    ...state.draft,
    collapsed: !state.draft.collapsed,
  };
  syncQuestionAnswerState(undefined, false);
});

questionExtraInput.addEventListener("input", () => {
  const state = currentQuestionState();
  if (!state) return;

  questionDrafts[activeQuestionIndex] = {
    ...state.draft,
    extraText: questionExtraInput.value,
  };
  if (renderedQuestionContent) {
    renderedQuestionContent = {
      ...renderedQuestionContent,
      extraText: questionExtraInput.value,
    };
  }
});

questionPreviousButton.addEventListener("click", () => {
  showQuestionAt(activeQuestionIndex - 1);
});

const focusCodexSessionWindow = async (
  threadId: string,
  button: HTMLButtonElement,
) => {
  const session = latestCodexSnapshot.sessions.find(
    (item) => item.id === threadId,
  );
  const cwdParts = session?.cwd?.split(/[\\/]/).filter(Boolean) ?? [];
  const cwdHint = cwdParts[cwdParts.length - 1];

  button.disabled = true;
  try {
    if (isTauriRuntime) {
      await invoke("focus_codex_window", {
        titleHint: session?.title ?? null,
        cwdHint: cwdHint ?? null,
      });
    }
  } catch (error) {
    console.error("Unable to focus the Codex window", error);
  } finally {
    button.disabled = false;
  }
};

questionOpenCodexButton.addEventListener("click", () => {
  const state = currentQuestionState();
  if (
    !state ||
    activeQuestionSource !== "codex" ||
    !state.question.readOnly ||
    !activeCodexQuestionThreadId
  ) {
    return;
  }

  void focusCodexSessionWindow(
    activeCodexQuestionThreadId,
    questionOpenCodexButton,
  );
});

questionRejectButton.addEventListener("click", async () => {
  const review = activeOpenCodeReview;
  if (
    activeQuestionSource !== "opencode" ||
    review?.reviewType !== "question"
  ) {
    return;
  }
  questionRejectButton.disabled = true;
  questionSubmitButton.disabled = true;
  let submittedToOpenCode = false;
  try {
    await invoke("reject_opencode_question", {
      pluginInstanceId: review.pluginInstanceId,
      sessionId: review.sessionId,
      requestId: review.requestId,
    });
    dismissedOpenCodeReviewId = undefined;
    submittedOpenCodeReviewId = openCodeReviewKey(review);
    followupOpenCodeReviewId = undefined;
    setQuestionSubmitStatus(translate("正在回传回答…"));
    submittedToOpenCode = true;
  } catch (error) {
    console.error("Unable to reject the OpenCode question", error);
    const message = error instanceof Error ? error.message : String(error);
    setQuestionSubmitStatus(`${translate("回传失败：")}${message}`, "error");
  } finally {
    if (!submittedToOpenCode) {
      questionSubmitButton.disabled = false;
      questionRejectButton.disabled = false;
    }
  }
});

questionNextButton.addEventListener("click", () => {
  const state = currentQuestionState();
  if (
    !state ||
    (state.question.readOnly
      ? activeQuestionIndex >= state.request.questions.length - 1
      : !questionActions(
          activeQuestionIndex,
          state.request.questions.length,
          state.draft,
        ).includes("next"))
  ) {
    return;
  }

  showQuestionAt(activeQuestionIndex + 1);
});

questionSubmitButton.addEventListener("click", async () => {
  const state = currentQuestionState();
  if (
    !state ||
    state.question.readOnly ||
    !questionActions(
      activeQuestionIndex,
      state.request.questions.length,
      state.draft,
    ).includes("submit")
  ) {
    return;
  }

  const submission = createLocalQuestionSubmission(
    state.request,
    questionDrafts,
  );
  const returnView = returnViewForReview(questionOriginView);
  questionSubmitButton.disabled = true;
  let submittedToOpenCode = false;

  try {
    if (activeQuestionSource === "opencode") {
      const review = activeOpenCodeReview;
      if (review?.reviewType !== "question") return;
      dismissedOpenCodeReviewId = undefined;
      await invoke("submit_opencode_question", {
        pluginInstanceId: review.pluginInstanceId,
        sessionId: review.sessionId,
        requestId: review.requestId,
        answers: submission.answers.map((answer) => [
          ...answer.selectedOptionLabels.filter((label) => label !== "其他"),
          ...(answer.extraText?.trim() ? [answer.extraText.trim()] : []),
        ]),
      });
      submittedOpenCodeReviewId = openCodeReviewKey(review);
      followupOpenCodeReviewId = undefined;
      questionRejectButton.disabled = true;
      setQuestionSubmitStatus(translate("正在回传回答…"));
      submittedToOpenCode = true;
    } else {
      await invoke("submit_claude_question_answer", {
        requestId: submission.requestId,
        answers: submission.answers,
      });
    }
    if (submittedToOpenCode) return;
    if (activeQuestionSource === "claude") {
      localQuestionSubmissions.set(state.request.id, {
        ...submission,
        contentSignature: questionRequestContentSignature(state.request),
      });
      locallySubmittedQuestionRequestId = state.request.id;
    }
    clearActiveQuestionRequest();
    questionOriginView = undefined;
    switchContentView(returnView);
  } catch (error) {
    console.error("Unable to submit question answers", error);
    if (activeQuestionSource === "opencode") {
      const message = error instanceof Error ? error.message : String(error);
      setQuestionSubmitStatus(`${translate("回传失败：")}${message}`, "error");
    }
  } finally {
    if (!submittedToOpenCode) {
      questionSubmitButton.disabled = false;
      if (activeQuestionSource === "opencode")
        questionRejectButton.disabled = false;
    }
  }
});

const renderPermissionRequest = (request: ClaudePermissionRequest) => {
  if (request.id === renderedPermissionRequestId) return;

  renderedPermissionRequestId = request.id;
  activePermissionRequest = request;
  permissionTool.textContent = request.toolName;
  permissionTool.title = request.toolName;
  permissionSummaryText.textContent = request.summary;
  permissionSummaryText.title = request.summary;

  const allowLabel = permissionAllowButton.querySelector(
    ".question-option__label",
  );
  const allowDescription = permissionAllowButton.querySelector(
    ".question-option__description",
  );
  permissionAlwaysAllowButton.hidden = !request.canAlwaysAllow;
  if (allowLabel) {
    allowLabel.textContent = request.canAlwaysAllow ? "允许一次" : "允许";
  }
  if (allowDescription) {
    allowDescription.textContent = request.canAlwaysAllow
      ? "仅执行这一次，不保存规则"
      : "执行这个工具调用";
  }

  if (request.cwd) {
    permissionCwd.hidden = false;
    permissionCwd.textContent = `目录：${request.cwd}`;
    permissionCwd.title = request.cwd;
  } else {
    permissionCwd.hidden = true;
    permissionCwd.textContent = "";
    permissionCwd.title = "";
  }
  permissionSubmitStatus.hidden = true;
  permissionSubmitStatus.textContent = "";
  delete permissionSubmitStatus.dataset.state;
};

const clearActivePermissionRequest = () => {
  renderedPermissionRequestId = undefined;
  activePermissionRequest = undefined;
  permissionSummaryText.textContent = "";
  permissionCwd.textContent = "";
  permissionCwd.hidden = true;
  permissionAlwaysAllowButton.hidden = false;
  permissionSubmitStatus.hidden = true;
  permissionSubmitStatus.textContent = "";
  delete permissionSubmitStatus.dataset.state;
};

const setPermissionSubmitStatus = (
  message: string | undefined,
  state: "pending" | "success" | "error" = "pending",
) => {
  permissionSubmitStatus.textContent = message ?? "";
  permissionSubmitStatus.hidden = !message;
  if (message) permissionSubmitStatus.dataset.state = state;
  else delete permissionSubmitStatus.dataset.state;
};

const syncOpenCodePermissionStatus = (
  review: OpenCodeNativePermissionReview | OpenCodeStrictToolGateReview,
) => {
  const alwaysLabel = permissionAlwaysAllowButton.querySelector(
    ".question-option__label",
  );
  const alwaysDescription = permissionAlwaysAllowButton.querySelector(
    ".question-option__description",
  );
  const isStrictGate = review.reviewType === "strictToolGate";
  if (alwaysLabel) {
    alwaysLabel.textContent = isStrictGate ? "本会话允许相同调用" : "始终允许";
  }
  if (alwaysDescription) {
    alwaysDescription.textContent = isStrictGate
      ? "仅匹配相同工具和参数范围"
      : "执行并记住，之后不再询问";
  }
  if (review.submitting) {
    setPermissionSubmitStatus(translate("正在回传决定…"));
  } else if (review.submissionError) {
    setPermissionSubmitStatus(
      `${translate("回传失败：")}${review.submissionError}`,
      "error",
    );
  } else if (openCodeReviewKey(review) === followupOpenCodeReviewId) {
    setPermissionSubmitStatus(
      translate("上一条已处理，这是新的审批请求"),
      "success",
    );
  } else {
    setPermissionSubmitStatus(undefined);
  }
};

const setPermissionButtonsDisabled = (disabled: boolean) => {
  permissionAllowButton.disabled = disabled;
  permissionAlwaysAllowButton.disabled = disabled;
  permissionDenyButton.disabled = disabled;
};

type PermissionDecision = "allow" | "allowAlways" | "deny";

const submitPermissionDecision = async (decision: PermissionDecision) => {
  const request = activePermissionRequest;
  if (!request) return;

  const returnView = returnViewForReview(permissionOriginView);
  setPermissionButtonsDisabled(true);
  let submittedToOpenCode = false;

  try {
    if (activePermissionSource === "codex") {
      await invoke("codex_respond_approval", {
        requestId: request.id,
        decision:
          decision === "allowAlways"
            ? "acceptForSession"
            : decision === "allow"
              ? "accept"
              : "decline",
      });
    } else if (activePermissionSource === "opencode") {
      const review = activeOpenCodeReview;
      if (review?.reviewType === "nativePermission") {
        await invoke("submit_opencode_permission", {
          pluginInstanceId: review.pluginInstanceId,
          sessionId: review.sessionId,
          requestId: review.requestId,
          action:
            decision === "allowAlways"
              ? "always"
              : decision === "allow"
                ? "once"
                : "reject",
          message: null,
        });
      } else if (review?.reviewType === "strictToolGate") {
        await invoke("submit_opencode_tool_gate", {
          pluginInstanceId: review.pluginInstanceId,
          sessionId: review.sessionId,
          reviewId: review.reviewId,
          action:
            decision === "allowAlways"
              ? "allowSession"
              : decision === "allow"
                ? "allowOnce"
                : "reject",
        });
      } else {
        return;
      }
      submittedOpenCodeReviewId = openCodeReviewKey(review);
      followupOpenCodeReviewId = undefined;
      submittedToOpenCode = true;
    } else {
      await invoke("submit_claude_permission_decision", {
        requestId: request.id,
        decision,
      });
    }
    if (submittedToOpenCode) {
      setPermissionSubmitStatus(translate("正在回传决定…"));
      return;
    }
    if (activePermissionSource === "codex") {
      dismissedCodexInteractionId = request.id;
    } else {
      locallySubmittedPermissionRequestId = request.id;
    }
    clearActivePermissionRequest();
    permissionOriginView = undefined;
    switchContentView(returnView);
  } catch (error) {
    console.error("Unable to submit the permission decision", error);
    if (activePermissionSource === "opencode") {
      const message = error instanceof Error ? error.message : String(error);
      setPermissionSubmitStatus(
        `${translate("回传失败：")}${message}`,
        "error",
      );
    }
  } finally {
    if (!submittedToOpenCode) setPermissionButtonsDisabled(false);
  }
};

permissionBackButton.addEventListener("click", () => {
  if (!activePermissionRequest) return;

  if (activePermissionSource === "codex") {
    dismissedCodexInteractionId = activePermissionRequest.id;
  } else if (activePermissionSource === "opencode") {
    dismissedOpenCodeReviewId = activePermissionRequest.id;
  } else {
    manuallyHiddenPermissionRequestId = activePermissionRequest.id;
  }
  const returnView = returnViewForReview(permissionOriginView);
  permissionOriginView = undefined;
  switchContentView(returnView);

  window.requestAnimationFrame(() => {
    const returnFocusTarget =
      returnView === "detail"
        ? sessionDetailBack
        : sessionList.querySelector<HTMLButtonElement>(".session-button");
    returnFocusTarget?.focus({ preventScroll: true });
  });
});

permissionAllowButton.addEventListener("click", () => {
  void submitPermissionDecision("allow");
});

permissionAlwaysAllowButton.addEventListener("click", () => {
  void submitPermissionDecision("allowAlways");
});

permissionDenyButton.addEventListener("click", () => {
  void submitPermissionDecision("deny");
});

const syncPlanSourceControls = () => {
  const isCodexPlan = activePlanSource === "codex";
  const isClaudePlan = activePlanSource === "claude";
  planView.dataset.planSource = activePlanSource;
  planAutoButton.hidden = !isClaudePlan;
  planAutoRememberButton.hidden = !isClaudePlan;
  planCustomInput.hidden = !isClaudePlan;
  planCustomSubmitButton.hidden = !isClaudePlan;
  planOpenCodexButton.hidden = !isCodexPlan;
  planCustomInput.placeholder = translate("请输入需要修改的内容");
  planCustomInput.setAttribute(
    "aria-label",
    translate("自定义指令"),
  );
  planCustomSubmitButton.textContent = translate("提交");
  planActionBadge.textContent = isCodexPlan
    ? "在原 Codex 中选择"
    : "点击后立即回传";
};

const renderPlanRequest = (
  request: ClaudePlanRequest,
  source: "claude" | "codex" = "claude",
  codexThreadId?: string,
) => {
  const unchanged =
    request.id === renderedPlanRequestId && activePlanSource === source;
  activePlanSource = source;
  activeCodexPlanThreadId = source === "codex" ? codexThreadId : undefined;
  syncPlanSourceControls();
  if (unchanged) return;

  renderedPlanRequestId = request.id;
  activePlanRequest = request;
  planTool.textContent = request.toolName;
  planTool.title = request.toolName;
  planSummaryText.innerHTML = renderPlanMarkdown(request.plan);
  planFullText.innerHTML = renderPlanMarkdown(request.plan);
  planCustomInput.value = "";

  if (request.cwd) {
    planCwd.hidden = false;
    planCwd.textContent = `目录：${request.cwd}`;
    planCwd.title = request.cwd;
  } else {
    planCwd.hidden = true;
    planCwd.textContent = "";
    planCwd.title = "";
  }

  refreshPlanPreviewClippedState();
};

const clearActivePlanRequest = () => {
  renderedPlanRequestId = undefined;
  activePlanRequest = undefined;
  activeCodexPlanThreadId = undefined;
  activePlanSource = "claude";
  syncPlanSourceControls();
  planSummaryText.innerHTML = "";
  planFullText.innerHTML = "";
  planPreview.dataset.clipped = "false";
  planPreview.removeAttribute("aria-describedby");
  planPreview.removeAttribute("tabindex");
  planCwd.textContent = "";
  planCwd.hidden = true;
  planCustomInput.value = "";
  setPlanSubmitStatus(undefined);
};

const setPlanButtonsDisabled = (disabled: boolean) => {
  planAutoButton.disabled = disabled;
  planAutoRememberButton.disabled = disabled;
  planCustomInput.disabled = disabled;
  planCustomSubmitButton.disabled = disabled;
  planOpenCodexButton.disabled = disabled;
};

const setPlanSubmitStatus = (
  message: string | undefined,
  state: "pending" | "success" | "error" = "pending",
) => {
  planSubmitStatus.textContent = message ?? "";
  planSubmitStatus.hidden = !message;
  if (message) planSubmitStatus.dataset.state = state;
  else delete planSubmitStatus.dataset.state;
};

type PlanExecutionMode = "auto";

const submitPlanDecision = async (mode: PlanExecutionMode, note?: string) => {
  const request = activePlanRequest;
  if (!request || activePlanSource !== "claude") return;

  const returnView = returnViewForReview(planOriginView);
  setPlanButtonsDisabled(true);

  try {
    await invoke("submit_claude_plan_decision", {
      requestId: request.id,
      mode,
      note: note?.trim() || null,
    });
    locallySubmittedPlanRequestId = request.id;
    clearActivePlanRequest();
    planOriginView = undefined;
    switchContentView(returnView);
  } catch (error) {
    console.error("Unable to submit the plan decision to Claude Code", error);
  } finally {
    setPlanButtonsDisabled(false);
  }
};

planBackButton.addEventListener("click", () => {
  if (!activePlanRequest) return;

  if (activePlanSource === "codex") {
    dismissedCodexInteractionId = activePlanRequest.id;
  } else {
    manuallyHiddenPlanRequestId = activePlanRequest.id;
  }
  const returnView = returnViewForReview(planOriginView);
  planOriginView = undefined;
  switchContentView(returnView);

  window.requestAnimationFrame(() => {
    const returnFocusTarget =
      returnView === "detail"
        ? sessionDetailBack
        : sessionList.querySelector<HTMLButtonElement>(".session-button");
    returnFocusTarget?.focus({ preventScroll: true });
  });
});

planAutoButton.addEventListener("click", () => {
  void submitPlanDecision("auto");
});

planAutoRememberButton.addEventListener("click", () => {
  void submitPlanDecision("auto");
});

planCustomSubmitButton.addEventListener("click", () => {
  void submitPlanDecision("auto", planCustomInput.value);
});

planCustomInput.addEventListener("keydown", (event) => {
  if (activePlanSource !== "claude") return;
  if (event.key !== "Enter" || event.isComposing) return;
  event.preventDefault();
  planCustomSubmitButton.click();
});

planOpenCodexButton.addEventListener("click", () => {
  if (activePlanSource !== "codex" || !activeCodexPlanThreadId) return;
  void focusCodexSessionWindow(activeCodexPlanThreadId, planOpenCodexButton);
});

const createSessionProductIcon = (
  product: SessionProduct,
): HTMLImageElement => {
  const image = document.createElement("img");
  image.src = product.iconUrl;
  image.width = 18;
  image.height = 18;
  image.alt = "";
  image.draggable = false;
  image.setAttribute("aria-hidden", "true");
  return image;
};

const visibleSessionProducts = () =>
  sessionProducts.filter(
    (product) =>
      product.id === "all" ||
      installedSessionProductIds.has(product.id as SessionSourceProductId),
  );

const selectedSessionProduct = () =>
  visibleSessionProducts().find(
    (product) => product.id === selectedSessionProductId,
  ) ?? sessionProducts[0];

const sessionProductOptionButtons = () =>
  Array.from(
    sessionProductMenu.querySelectorAll<HTMLButtonElement>(
      ".session-product__option",
    ),
  );

const updateSessionProductTrigger = () => {
  const product = selectedSessionProduct();
  sessionProductTitle.textContent = product.triggerLabel;
  sessionProductIcon.dataset.productKind = product.kind;
  sessionProductIcon.replaceChildren(createSessionProductIcon(product));
  sessionProductTrigger.setAttribute(
    "aria-label",
    `筛选会话来源，当前为${product.optionLabel}`,
  );
};

const renderSessionProductMenu = () => {
  const visibleProducts = visibleSessionProducts();
  const orderedProducts = [
    selectedSessionProduct(),
    ...visibleProducts.filter(
      (product) => product.id !== selectedSessionProductId,
    ),
  ];
  const options = orderedProducts.map((product, index) => {
    const option = document.createElement("button");
    option.className = "session-product__option";
    option.type = "button";
    option.role = "option";
    option.dataset.productId = product.id;
    option.setAttribute(
      "aria-selected",
      String(product.id === selectedSessionProductId),
    );

    const mark = document.createElement("span");
    mark.className = "session-product__option-mark";
    mark.dataset.productKind = product.kind;
    mark.setAttribute("aria-hidden", "true");
    mark.append(createSessionProductIcon(product));

    const label = document.createElement("span");
    label.className = "session-product__option-label";
    label.textContent = product.optionLabel;

    const chevronSlot = document.createElement("span");
    chevronSlot.className = "session-product__option-chevron-slot";
    chevronSlot.setAttribute("aria-hidden", "true");

    if (index === 0) {
      const chevron = sessionProductTrigger
        .querySelector<SVGSVGElement>(".session-product__chevron")
        ?.cloneNode(true) as SVGSVGElement | undefined;
      if (chevron) {
        chevron.classList.add("session-product__option-chevron");
        chevronSlot.append(chevron);
      }
    }
    option.append(mark, label, chevronSlot);
    option.addEventListener("click", () => {
      void selectSessionProductOption(product, option);
    });
    return option;
  });

  sessionProductMenu.replaceChildren(...options);
  sessionProductMenu.style.setProperty(
    "--session-product-menu-height",
    `${orderedProducts.length * 30 + 2}px`,
  );
};

const syncSessionProductWidth = () => {
  renderSessionProductMenu();
  sessionProductMenu.dataset.measuring = "true";
  sessionProductMenu.hidden = false;

  const widestOptionWidth = Math.ceil(
    sessionProductMenu.getBoundingClientRect().width,
  );
  if (widestOptionWidth > 0) {
    sessionProduct.style.setProperty(
      "--session-product-width",
      `${widestOptionWidth}px`,
    );
  }

  delete sessionProductMenu.dataset.measuring;
  sessionProductMenu.hidden = true;
};

const animateSessionProductOptionToFirst = async (
  selectedOption: HTMLButtonElement,
) => {
  const options = sessionProductOptionButtons();
  const firstOption = options[0];
  if (!firstOption || firstOption === selectedOption) return;

  const previousTops = new Map(
    options.map((option) => [option, option.getBoundingClientRect().top]),
  );
  const chevron = firstOption.querySelector<SVGSVGElement>(
    ".session-product__option-chevron",
  );
  const selectedChevronSlot = selectedOption.querySelector<HTMLElement>(
    ".session-product__option-chevron-slot",
  );
  if (chevron && selectedChevronSlot) {
    selectedChevronSlot.replaceChildren(chevron);
  }
  sessionProductMenu.prepend(selectedOption);

  const animations = sessionProductOptionButtons()
    .map((option) => {
      const previousTop = previousTops.get(option);
      if (previousTop === undefined) return undefined;
      const offsetY = previousTop - option.getBoundingClientRect().top;
      if (Math.abs(offsetY) < 0.5) return undefined;
      return option.animate(
        [
          { transform: `translateY(${offsetY}px)` },
          { transform: "translateY(0)" },
        ],
        {
          duration: SESSION_PRODUCT_OPTION_REORDER_MS,
          easing: "cubic-bezier(0.2, 0.7, 0.35, 0.95)",
        },
      );
    })
    .filter((animation): animation is Animation => animation !== undefined);

  await Promise.all(
    animations.map((animation) => animation.finished.catch(() => undefined)),
  );
};

const syncProductVisibility = () => {
  const showClaude =
    installedSessionProductIds.has("claude-code") &&
    (selectedSessionProductId === "all" ||
      selectedSessionProductId === "claude-code");
  const showCodex =
    installedSessionProductIds.has("codex") &&
    (selectedSessionProductId === "all" ||
      selectedSessionProductId === "codex");
  const showOpenCode =
    installedSessionProductIds.has("opencode") &&
    (selectedSessionProductId === "all" ||
      selectedSessionProductId === "opencode");
  claudeSessionCard.hidden = !showClaude;
  codexSessionCard.hidden = !showCodex;
  openCodeSessionCard.hidden = !showOpenCode;
  sessionSourceCards.dataset.filter = selectedSessionProductId;
  renderSessionSummary();
};

const sessionProductIdForHookAgent = (
  id: string,
): SessionSourceProductId | undefined => {
  if (id === "claudeCode") return "claude-code";
  if (id === "codex") return "codex";
  if (id === "openCode") return "opencode";
  return undefined;
};

const syncInstalledSessionProducts = (
  statuses: ReadonlyArray<{ id: string; hookInstalled: boolean }>,
) => {
  installedSessionProductIds.clear();
  for (const status of statuses) {
    if (!status.hookInstalled) continue;
    const productId = sessionProductIdForHookAgent(status.id);
    if (productId) installedSessionProductIds.add(productId);
  }

  if (
    selectedSessionProductId !== "all" &&
    !installedSessionProductIds.has(selectedSessionProductId)
  ) {
    selectedSessionProductId = "all";
  }

  updateSessionProductTrigger();
  syncProductVisibility();
  syncSessionProductWidth();
};

const selectSessionProductOption = async (
  product: SessionProduct,
  option: HTMLButtonElement,
) => {
  if (sessionProductSelectionPending) return;
  sessionProductSelectionPending = true;
  sessionProductMenu.dataset.reordering = "true";
  sessionProductMenu.setAttribute("aria-busy", "true");

  try {
    await animateSessionProductOptionToFirst(option);
    selectedSessionProductId = product.id;
    syncProductVisibility();
    for (const productOption of sessionProductOptionButtons()) {
      productOption.setAttribute(
        "aria-selected",
        String(productOption.dataset.productId === product.id),
      );
    }
    updateSessionProductTrigger();
    closeSessionProductMenu(true);
  } finally {
    sessionProductSelectionPending = false;
    delete sessionProductMenu.dataset.reordering;
    sessionProductMenu.removeAttribute("aria-busy");
  }
};

const closeSessionProductMenu = (restoreFocus = false) => {
  if (sessionProductMenu.hidden) return;
  if (sessionProductMenuCloseTimer !== undefined) {
    clearTimeout(sessionProductMenuCloseTimer);
    sessionProductMenuCloseTimer = undefined;
  }
  if (sessionProductMenuOpenFrame !== undefined) {
    cancelAnimationFrame(sessionProductMenuOpenFrame);
    sessionProductMenuOpenFrame = undefined;
  }
  sessionProductMenu.dataset.open = "false";
  sessionProductTrigger.setAttribute("aria-expanded", "false");
  if (restoreFocus) {
    sessionProductTrigger.focus({ preventScroll: true });
  }
  sessionProductMenuCloseTimer = setTimeout(() => {
    sessionProductMenu.hidden = true;
    sessionProductMenuCloseTimer = undefined;
  }, SESSION_PRODUCT_MENU_TRANSITION_MS + SESSION_PRODUCT_MENU_FADE_MS);
};

const openSessionProductMenu = (focusIndex = 0) => {
  if (sessionProductMenuCloseTimer !== undefined) {
    clearTimeout(sessionProductMenuCloseTimer);
    sessionProductMenuCloseTimer = undefined;
  }
  if (sessionProductMenuOpenFrame !== undefined) {
    cancelAnimationFrame(sessionProductMenuOpenFrame);
    sessionProductMenuOpenFrame = undefined;
  }
  renderSessionProductMenu();
  sessionProductMenu.dataset.open = "false";
  sessionProductMenu.hidden = false;
  sessionProductMenuOpenFrame = requestAnimationFrame(() => {
    sessionProductMenuOpenFrame = undefined;
    sessionProductTrigger.setAttribute("aria-expanded", "true");
    sessionProductMenu.dataset.open = "true";
  });
  const options = sessionProductOptionButtons();
  options[Math.max(0, Math.min(focusIndex, options.length - 1))]?.focus({
    preventScroll: true,
  });
};

sessionProductTrigger.addEventListener("click", () => {
  if (sessionProductMenu.hidden) {
    openSessionProductMenu();
  } else {
    closeSessionProductMenu(true);
  }
});

sessionProductTrigger.addEventListener("keydown", (event) => {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  openSessionProductMenu(
    event.key === "ArrowUp" ? sessionProducts.length - 1 : 0,
  );
});

sessionProductMenu.addEventListener("keydown", (event) => {
  if (sessionProductSelectionPending) {
    event.preventDefault();
    return;
  }

  if (event.key === "Escape") {
    event.preventDefault();
    closeSessionProductMenu(true);
    return;
  }

  if (event.key === "Tab") {
    closeSessionProductMenu();
    return;
  }

  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const options = sessionProductOptionButtons();
  const currentIndex = options.indexOf(
    document.activeElement as HTMLButtonElement,
  );
  const nextIndex =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? options.length - 1
        : event.key === "ArrowDown"
          ? (currentIndex + 1) % options.length
          : (currentIndex - 1 + options.length) % options.length;
  options[nextIndex]?.focus({ preventScroll: true });
});

document.addEventListener("pointerdown", (event) => {
  if (sessionProductMenu.hidden) return;
  if (sessionProductSelectionPending) return;
  if (sessionProductTrigger.contains(event.target as Node)) return;
  if (sessionProductMenu.contains(event.target as Node)) return;
  closeSessionProductMenu();
});

updateSessionProductTrigger();
syncSessionProductWidth();
claudeSessionCardIcon.src = claudeCodeIconUrl;
codexSessionCardIcon.src = codexIconUrl;
openCodeSessionCardIcon.src = openCodeIconUrl;

claudeConnectionStatus.addEventListener("click", () => {
  if (claudeConnectionStatus.dataset.connected === "true") return;
  claudeConnectionStatus.disabled = true;
  claudeConnectionStatus.dataset.pending = "true";
  setSourceStatusLabel(claudeConnectionStatus, "修复中");
  void invoke("claude_hook_install")
    .catch((error: unknown) => {
      console.error("Unable to reinstall the Claude Code Hook", error);
    })
    .finally(() => {
      delete claudeConnectionStatus.dataset.pending;
      void refreshClaudeSessions();
    });
});

const activityStatusLabel = (status: string) => {
  switch (status) {
    case "running":
      return "执行中";
    case "failed":
      return "失败";
    default:
      return "已完成";
  }
};

const createPixelStatusSvg = (status: string, className: string) => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.classList.add(className);
  svg.dataset.status = status;
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.setAttribute("shape-rendering", "crispEdges");

  const pixels = [
    [6, 1, 4, 3],
    [3, 4, 4, 4],
    [9, 4, 4, 4],
    [1, 8, 4, 4],
    [6, 8, 4, 4],
    [11, 8, 4, 4],
    [6, 12, 4, 3],
  ];
  pixels.forEach(([x, y, width, height], index) => {
    const pixel = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "rect",
    );
    pixel.classList.add("pixel-status__pixel");
    pixel.dataset.pixel = String(index);
    pixel.setAttribute("x", String(x));
    pixel.setAttribute("y", String(y));
    pixel.setAttribute("width", String(width));
    pixel.setAttribute("height", String(height));
    svg.append(pixel);
  });
  return svg;
};

const sessionLiveStatusIcon = createPixelStatusSvg("idle", "pixel-status");
sessionLiveStatus.replaceChildren(sessionLiveStatusIcon);

const latestUnifiedSessions = (): UnifiedSession[] => [
  ...latestSessions,
  ...latestCodexSnapshot.sessions,
  ...latestOpenCodeSnapshot.sessions,
];

const collapsedPanelHeight = () =>
  liveCollapsedHeightActive ? LIVE_COLLAPSED_HEIGHT : COLLAPSED_PANEL_HEIGHT;
const collapsedPanelCornerProgress = () =>
  liveCollapsedHeightActive ? LIVE_COLLAPSED_CORNER_PROGRESS : 0;

const syncCollapsedPanelSize = (): Promise<void> => {
  if (!isTauriRuntime) return Promise.resolve();
  if (document.documentElement.dataset.panelState !== "collapsed") {
    return Promise.resolve();
  }

  beginPanelShapeTracking();
  return invoke<void>("set_panel_expanded", {
    expanded: false,
    panelWidth: currentNativePanelWidth(),
    interfaceScale: currentInterfaceScale(),
    height: expandedPanelHeight * currentInterfaceScale(),
    collapsedHeight: collapsedPanelHeight() * currentInterfaceScale(),
    collapsedCornerProgress: collapsedPanelCornerProgress(),
    animateHeight: true,
  })
    .catch((error: unknown) => {
      console.error("Unable to resize the collapsed CodeCraft panel", error);
    })
    .finally(() => {
      endPanelShapeTracking();
    });
};

const updateCollapsedLiveView = () => {
  const collapsed = document.documentElement.dataset.panelState === "collapsed";
  const collapseSource = document.documentElement.dataset.collapseSource;
  const liveSession = primaryUnifiedLiveSession(latestUnifiedSessions());
  const eligible =
    (collapseSource === "auto" || collapseSource === "mini") &&
    liveSession !== undefined;
  const liveShown = collapsed && eligible;
  const heightChanged = eligible !== liveCollapsedHeightActive;
  liveCollapsedHeightActive = eligible;

  document.documentElement.toggleAttribute("data-panel-live", liveShown);
  finishCollapsedWorkingFlowIfHidden();
  sessionLiveView.hidden = !liveShown;
  sessionLiveView.setAttribute("aria-hidden", String(!liveShown));
  panel.setAttribute("aria-hidden", String(collapsed && !liveShown));
  panelViewStage.setAttribute("aria-hidden", String(collapsed));
  collapseHandle.tabIndex = collapsed ? -1 : 0;

  if (!liveSession) {
    if (heightChanged) void syncCollapsedPanelSize();
    return;
  }

  const statusText = unifiedSessionLiveStatusText(liveSession);
  const visualStatus = unifiedSessionVisualStatus(liveSession);
  sessionLiveTitle.textContent = liveSession.title;
  sessionLiveTitle.title = liveSession.title;
  sessionLiveStatusIcon.dataset.status = visualStatus;
  sessionLiveStatusTextElement.dataset.status = visualStatus;
  sessionLiveStatusTextElement.textContent = statusText;

  sessionLiveView.setAttribute(
    "aria-label",
    `${statusText} · ${liveSession.title}`,
  );

  if (heightChanged) void syncCollapsedPanelSize();
};

const waitForStartupPhase = (durationMs: number) =>
  new Promise<void>((resolve) => {
    window.setTimeout(resolve, durationMs);
  });

const resizeStartupPanel = async (
  expanded: boolean,
  animateHeight: boolean,
) => {
  if (!isTauriRuntime) return;

  beginPanelShapeTracking();
  try {
    await invoke<void>("set_panel_expanded", {
      expanded,
      panelWidth: currentNativePanelWidth(),
      interfaceScale: currentInterfaceScale(),
      height: STARTUP_PANEL_HEIGHT * currentInterfaceScale(),
      collapsedHeight: collapsedPanelHeight() * currentInterfaceScale(),
      collapsedCornerProgress: collapsedPanelCornerProgress(),
      animateHeight,
    });
  } finally {
    endPanelShapeTracking();
  }
};

const runStartupSequence = async () => {
  const animationSpeed = currentAnimationSpeed();
  const startupAnimationMode = effectiveStartupAnimationMode(
    appearanceSettings,
    systemMotionPreference.matches,
  );
  if (startupAnimationMode === "none") {
    updateCollapsedLiveView();
    return;
  }

  const phaseDuration = (durationMs: number) =>
    Math.max(1, Math.round(durationMs / animationSpeed));

  startupAppIcon.src = codeCraftIconUrl;
  startupSequence.setAttribute("aria-hidden", "false");
  panel.setAttribute("aria-hidden", "true");
  panelViewStage.setAttribute("aria-hidden", "true");
  rootElement.dataset.panelState = "startup";
  rootElement.dataset.startupPhase = "reveal";
  syncPanelShape();

  try {
    await waitForStartupPhase(phaseDuration(STARTUP_RAIL_DURATION_MS));

    if (startupAnimationMode === "full") {
      rootElement.dataset.startupPhase = "expand";
      await resizeStartupPanel(true, true);
      rootElement.dataset.startupPhase = "intro";
      await waitForStartupPhase(phaseDuration(STARTUP_INTRO_DURATION_MS));
      rootElement.dataset.startupPhase = "exit";
      await waitForStartupPhase(phaseDuration(STARTUP_EXIT_DURATION_MS));
    }

    const liveSession = primaryUnifiedLiveSession(latestUnifiedSessions());
    const collapseSource = rootElement.dataset.collapseSource;
    liveCollapsedHeightActive =
      (collapseSource === "auto" || collapseSource === "mini") &&
      liveSession !== undefined;
    await resizeStartupPanel(false, startupAnimationMode === "full");
  } catch (error: unknown) {
    console.error("Unable to play the CodeCraft startup sequence", error);
    try {
      await resizeStartupPanel(false, false);
    } catch {
      // Keep the frontend state usable even if the native resize is unavailable.
    }
  } finally {
    rootElement.dataset.panelState = "collapsed";
    rootElement.removeAttribute("data-startup-phase");
    startupSequence.setAttribute("aria-hidden", "true");
    syncPanelShape();
    updateCollapsedLiveView();
  }
};

const syncCollapsedSessionState = () => {
  if (hasUnifiedWorkingSession(latestUnifiedSessions())) {
    startCollapsedWorking();
  } else {
    stopCollapsedWorking();
  }
  updateCollapsedLiveView();
};

const renderSessionDetail = (session: UnifiedSession) => {
  const signature = JSON.stringify([
    unifiedSessionKey(session),
    session.status,
    session.title,
    session.activities,
    session.outputs,
    isOpenCodeSession(session) ? session.pendingReviews : null,
  ]);
  if (signature === renderedDetailSignature) return;

  const previousSessionId = sessionDetailView.dataset.sessionId;
  const sessionKey = unifiedSessionKey(session);
  const outputWasAtEnd =
    previousSessionId !== sessionKey ||
    sessionOutput.scrollHeight -
      sessionOutput.scrollTop -
      sessionOutput.clientHeight <
      24;

  renderedDetailSignature = signature;
  sessionDetailView.dataset.sessionId = sessionKey;
  sessionDetailView.dataset.sessionStatus = session.status;
  sessionDetailTitle.textContent = session.title;
  sessionDetailTitle.title = session.title;
  sessionDetailStatus.dataset.status = session.status;
  sessionDetailStatus.replaceChildren(
    createPixelStatusSvg(session.status, "pixel-status"),
    document.createTextNode(unifiedSessionStatusLabel(session)),
  );

  const runningCount = session.activities.filter(
    (activity) => activity.status === "running",
  ).length;
  activitySummary.textContent =
    runningCount > 0
      ? `${runningCount} 项执行中`
      : `${session.activities.length} 项记录`;

  if (session.activities.length === 0) {
    const empty = document.createElement("li");
    empty.className = "detail-empty";
    empty.textContent = "等待 Claude 调用工具";
    sessionActivityList.replaceChildren(empty);
  } else {
    const items = [...session.activities].reverse().map((activity) => {
      const item = document.createElement("li");
      item.className = "activity-item";
      item.dataset.activityStatus = activity.status;

      const status = document.createElement("span");
      status.className = "activity-item__status";
      status.ariaHidden = "true";

      const statusText = document.createElement("span");
      statusText.className = "sr-only";
      statusText.textContent = activityStatusLabel(activity.status);

      const content = document.createElement("span");
      content.className = "activity-item__content";
      const tool = document.createElement("span");
      tool.className = "activity-item__tool";
      tool.textContent = activity.tool;
      const summary = document.createElement("span");
      summary.className = "activity-item__summary";
      summary.textContent = activity.summary;
      summary.title = activity.summary;
      content.append(tool, summary);

      const time = document.createElement("time");
      time.className = "activity-item__time";
      time.dateTime = new Date(activity.updatedAt).toISOString();
      time.textContent = formatSessionTime(activity.updatedAt);

      item.append(status, statusText, content, time);
      return item;
    });
    sessionActivityList.replaceChildren(...items);
  }

  if (session.outputs.length === 0) {
    const empty = document.createElement("p");
    empty.className = "detail-empty";
    empty.textContent = "等待 Claude 产生可读取的转录输出";
    sessionOutput.replaceChildren(empty);
  } else {
    const outputEntries = session.outputs.map((output) => {
      const entry = document.createElement("article");
      entry.className = "output-entry markdown-content";
      entry.dataset.outputId = output.id;
      entry.innerHTML = renderMarkdown(output.text);
      return entry;
    });
    sessionOutput.replaceChildren(...outputEntries);
  }

  if (outputWasAtEnd) {
    sessionOutput.scrollTop = sessionOutput.scrollHeight;
  }
};

const setSelectedSession = (sessionId: string) => {
  const session = latestSessions.find((item) => item.id === sessionId);
  if (!session) return;

  selectedSessionId = sessionId;
  selectedSessionSource = "claude";
  for (const button of sessionList.querySelectorAll<HTMLButtonElement>(
    ".session-button",
  )) {
    const selected = button.dataset.sessionId === selectedSessionId;
    button.setAttribute("aria-pressed", String(selected));
  }

  const entryView = sessionEntryView(session);
  if (entryView === "plan" && session.plan) {
    manuallyHiddenPlanRequestId = undefined;
    planOriginView = "sessions";
    renderPlanRequest(session.plan);
    switchContentView("plan");
    refreshPlanPreviewClippedState();
    planBackButton.focus({ preventScroll: true });
    return;
  }

  if (entryView === "permission" && session.permission) {
    activePermissionSource = "claude";
    activeCodexQuestionThreadId = undefined;
    manuallyHiddenPermissionRequestId = undefined;
    permissionOriginView = "sessions";
    renderPermissionRequest(session.permission);
    switchContentView("permission");
    permissionBackButton.focus({ preventScroll: true });
    return;
  }

  if (entryView === "question" && session.question) {
    activeQuestionSource = "claude";
    activeCodexQuestionThreadId = undefined;
    manuallyHiddenQuestionRequestId = undefined;
    questionOriginView = "sessions";
    renderQuestionRequest(session.question);
    switchContentView("question");
    return;
  }

  renderSessionDetail(session);
  switchContentView("detail");
  sessionDetailBack.focus({ preventScroll: true });
};

const setSelectedCodexSession = (sessionId: string) => {
  const session = latestCodexSnapshot.sessions.find(
    (item) => item.id === sessionId,
  );
  if (!session) return;
  selectedCodexSessionId = sessionId;
  selectedSessionSource = "codex";
  selectedSessionId = undefined;
  for (const button of codexSessionList.querySelectorAll<HTMLButtonElement>(
    ".session-button",
  )) {
    button.setAttribute(
      "aria-pressed",
      String(button.dataset.sessionId === sessionId),
    );
  }
  const interaction = codexInteractionFor(
    session,
    latestCodexSnapshot.interactions,
  );
  if (interaction && !interaction.resolved) {
    renderCodexInteraction(interaction, session, false);
    return;
  }
  renderSessionDetail(session);
  switchContentView("detail");
  sessionDetailBack.focus({ preventScroll: true });
};

const showOpenCodeReview = (
  session: OpenCodeSession,
  review: OpenCodeReview,
  shouldAutoReveal: boolean,
) => {
  activeOpenCodeReview = review;
  const reveal = (view: ReviewContentView) => {
    if (requestedContentView !== view) {
      rememberReviewOrigin(view);
      switchContentView(view);
    }
    if (!shouldAutoReveal || !collapseExpandSettings.approvalAutoExpand) return;
    void revealPanelForAttention().catch((error: unknown) => {
      console.error("Unable to reveal the OpenCode review", error);
    });
  };
  if (review.reviewType === "question") {
    activeQuestionSource = "opencode";
    renderQuestionRequest(openCodeQuestionRequest(review));
    syncOpenCodeQuestionStatus(review);
    questionSubmitButton.disabled = review.submitting;
    questionRejectButton.disabled = review.submitting;
    reveal("question");
    return;
  }
  activePermissionSource = "opencode";
  renderPermissionRequest(openCodePermissionRequest(review, session));
  syncOpenCodePermissionStatus(review);
  setPermissionButtonsDisabled(review.submitting);
  reveal("permission");
};

const setSelectedOpenCodeSession = (sessionKey: string) => {
  const session = latestOpenCodeSnapshot.sessions.find(
    (item) => openCodeSessionKey(item) === sessionKey,
  );
  if (!session) return;
  selectedOpenCodeSessionKey = sessionKey;
  selectedSessionSource = "opencode";
  selectedSessionId = undefined;
  selectedCodexSessionId = undefined;
  for (const button of openCodeSessionList.querySelectorAll<HTMLButtonElement>(
    ".session-button",
  )) {
    button.setAttribute(
      "aria-pressed",
      String(button.dataset.sessionKey === sessionKey),
    );
  }
  const review = openCodeReviewForSession(session);
  if (review) {
    showOpenCodeReview(session, review, false);
    return;
  }
  renderSessionDetail(session);
  switchContentView("detail");
  sessionDetailBack.focus({ preventScroll: true });
};

const renderSessionSummary = () => {
  const claudeActiveCount = latestSessions.filter(
    (session) => session.status !== "idle",
  ).length;
  const codexActiveCount = latestCodexSnapshot.sessions.filter(
    (session) => session.status !== "idle",
  ).length;
  const openCodeActiveCount = latestOpenCodeSnapshot.sessions.filter(
    (session) => session.status !== "idle",
  ).length;
  const activeCounts: Record<SessionSourceProductId, number> = {
    "claude-code": claudeActiveCount,
    codex: codexActiveCount,
    opencode: openCodeActiveCount,
  };
  const connectionStates: Record<SessionSourceProductId, boolean> = {
    "claude-code": latestClaudeSnapshot.connected,
    codex: latestCodexSnapshot.connected,
    opencode: isOpenCodeHookHealthy(),
  };
  const selectedSources =
    selectedSessionProductId === "all"
      ? [...installedSessionProductIds]
      : installedSessionProductIds.has(selectedSessionProductId)
        ? [selectedSessionProductId]
        : [];
  const count = selectedSources.reduce(
    (total, source) => total + activeCounts[source],
    0,
  );
  const connected =
    selectedSources.length > 0 &&
    selectedSources.every((source) => connectionStates[source]);
  integrationState.dataset.connected = String(connected);
  sessionSummary.textContent = `${count} 个活跃`;
};

const renderCodexSnapshot = (snapshot: CodexSnapshot) => {
  // Codex sessions have no source other than the CodeCraft hook. Keep the UI
  // fail-closed while Hook status is unknown or reports it uninstalled.
  if (isTauriRuntime && isCodexHookKnownUninstalled()) {
    snapshot = {
      ...snapshot,
      connected: false,
      integrationError: snapshot.integrationError ?? "Codex Hook 未安装",
      sessions: [],
      interactions: [],
    };
  }
  const visibleSessions = filterAutoCleanedSessions(
    filterDismissedSessions(
      snapshot.sessions,
      dismissedSessionKeys,
      (session) => `codex:${session.id}`,
      unifiedSessionIsRunning,
    ),
    sessionCleanupSettings,
  );
  if (visibleSessions.length !== snapshot.sessions.length) {
    const visibleSessionIds = new Set(
      visibleSessions.map((session) => session.id),
    );
    const expiredSessionIds = new Set(
      snapshot.sessions
        .filter((session) => !visibleSessionIds.has(session.id))
        .map((session) => session.id),
    );
    snapshot = {
      ...snapshot,
      sessions: visibleSessions,
      interactions: snapshot.interactions.filter(
        (interaction) => !expiredSessionIds.has(interaction.threadId),
      ),
    };
  }
  observeCodexSounds(snapshot.sessions, snapshot.interactions);
  latestCodexSnapshot = snapshot;
  if (
    !snapshot.sessions.some((session) => session.id === selectedCodexSessionId)
  ) {
    selectedCodexSessionId = undefined;
    if (selectedSessionSource === "codex") {
      renderedDetailSignature = undefined;
    }
  }
  renderUnifiedSessionList(
    codexSessionList,
    snapshot.sessions,
    codexSessionItems,
  );
  renderSessionSummary();
  syncCollapsedSessionState();

  const pendingEntry = codexPendingInteraction(
    snapshot.sessions,
    snapshot.interactions,
  );
  if (!pendingEntry) {
    if (
      requestedContentView === "question" &&
      activeQuestionSource === "codex"
    ) {
      const returnView = returnViewForReview(questionOriginView);
      questionOriginView = undefined;
      clearActiveQuestionRequest();
      switchContentView(returnView);
    } else if (
      requestedContentView === "permission" &&
      activePermissionSource === "codex"
    ) {
      const returnView = returnViewForReview(permissionOriginView);
      permissionOriginView = undefined;
      clearActivePermissionRequest();
      switchContentView(returnView);
    } else if (
      requestedContentView === "plan" &&
      activePlanSource === "codex"
    ) {
      const returnView = returnViewForReview(planOriginView);
      planOriginView = undefined;
      clearActivePlanRequest();
      switchContentView(returnView);
    }
    lastAutoRevealedCodexInteractionId = undefined;
    dismissedCodexInteractionId = undefined;
    return;
  }
  const { interaction: pending, session } = pendingEntry;
  if (pending.requestId === dismissedCodexInteractionId) return;
  const shouldAutoReveal =
    pending.requestId !== lastAutoRevealedCodexInteractionId;
  renderCodexInteraction(pending, session, shouldAutoReveal);
  if (shouldAutoReveal) lastAutoRevealedCodexInteractionId = pending.requestId;
};

const renderCodexInteraction = (
  interaction: CodexInteraction,
  session: CodexSession,
  shouldAutoReveal: boolean,
) => {
  const reveal = (view: ReviewContentView) => {
    if (requestedContentView !== view) {
      rememberReviewOrigin(view);
      switchContentView(view);
    }
    if (!shouldAutoReveal || !collapseExpandSettings.approvalAutoExpand) return;
    void revealPanelForAttention().catch((error: unknown) => {
      console.error("Unable to reveal the Codex interaction", error);
    });
  };
  if (interaction.kind === "userInput") {
    clearActivePermissionRequest();
    activeQuestionSource = "codex";
    activeCodexQuestionThreadId = interaction.threadId;
    renderQuestionRequest(codexQuestionRequest(interaction));
    reveal("question");
    return;
  }
  if (interaction.kind === "plan") {
    clearActiveQuestionRequest();
    clearActivePermissionRequest();
    renderPlanRequest(
      codexPlanRequest(interaction, session.cwd),
      "codex",
      interaction.threadId,
    );
    reveal("plan");
    return;
  }
  activePermissionSource = "codex";
  renderPermissionRequest({
    id: interaction.requestId,
    toolName: interaction.title || "Codex Hook 工具调用",
    summary: interaction.detail || "Codex Hook 请求执行操作",
    cwd: session.cwd,
    canAlwaysAllow: interaction.allowSession,
    capturedAt: interaction.capturedAt,
  });
  reveal("permission");
};

const renderOpenCodeSnapshot = (snapshot: OpenCodeSnapshot) => {
  snapshot = {
    ...snapshot,
    sessions: filterAutoCleanedSessions(
      filterDismissedSessions(
        snapshot.sessions,
        dismissedSessionKeys,
        (session) => `opencode:${openCodeSessionKey(session)}`,
        unifiedSessionIsRunning,
      ),
      sessionCleanupSettings,
    ),
  };
  observeOpenCodeSounds(snapshot.sessions);
  latestOpenCodeSnapshot = snapshot;
  syncOpenCodeHookStatus();

  if (
    !snapshot.sessions.some(
      (session) => openCodeSessionKey(session) === selectedOpenCodeSessionKey,
    )
  ) {
    selectedOpenCodeSessionKey = undefined;
    if (selectedSessionSource === "opencode")
      renderedDetailSignature = undefined;
  }
  renderUnifiedSessionList(
    openCodeSessionList,
    snapshot.sessions,
    openCodeSessionItems,
  );
  const selectedOpenCodeSession = snapshot.sessions.find(
    (session) => openCodeSessionKey(session) === selectedOpenCodeSessionKey,
  );
  if (
    selectedSessionSource === "opencode" &&
    requestedContentView === "detail" &&
    selectedOpenCodeSession
  ) {
    renderSessionDetail(selectedOpenCodeSession);
  }
  renderSessionSummary();
  syncCollapsedSessionState();

  const pendingReveal = openCodePendingReveal(
    snapshot.sessions,
    lastAutoRevealedOpenCodeReviewId,
    dismissedOpenCodeReviewId,
  );
  const pendingEntry = pendingReveal;
  if (submittedOpenCodeReviewId) {
    const submittedStillPending = snapshot.sessions.some((session) =>
      openCodeReviewsForSession(session).some(
        (review) => openCodeReviewKey(review) === submittedOpenCodeReviewId,
      ),
    );
    if (!submittedStillPending) {
      submittedOpenCodeReviewId = undefined;
      followupOpenCodeReviewId = pendingEntry
        ? openCodeReviewKey(pendingEntry.review)
        : undefined;
    }
  }
  if (!pendingEntry) {
    if (
      (requestedContentView === "question" &&
        activeQuestionSource === "opencode") ||
      (requestedContentView === "permission" &&
        activePermissionSource === "opencode")
    ) {
      const origin =
        requestedContentView === "question"
          ? questionOriginView
          : permissionOriginView;
      const returnView = returnViewForReview(origin);
      clearActiveQuestionRequest();
      clearActivePermissionRequest();
      activeOpenCodeReview = undefined;
      switchContentView(returnView);
    }
    dismissedOpenCodeReviewId = undefined;
    lastAutoRevealedOpenCodeReviewId = undefined;
    followupOpenCodeReviewId = undefined;
    return;
  }

  const reviewId = pendingReveal.reviewId;
  if (pendingReveal.dismissed) return;
  const shouldAutoReveal = pendingReveal.shouldAutoReveal;
  showOpenCodeReview(
    pendingEntry.session,
    pendingEntry.review,
    shouldAutoReveal,
  );
  if (shouldAutoReveal) lastAutoRevealedOpenCodeReviewId = reviewId;
};

const refreshOpenCodeSessions = async () => {
  if (refreshingOpenCodeSessions || !isTauriRuntime) return;
  refreshingOpenCodeSessions = true;
  try {
    const snapshot = await invoke<OpenCodeSnapshot>("list_opencode_sessions");
    renderOpenCodeSnapshot(snapshot);
    lastOpenCodeRefreshError = undefined;
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    renderOpenCodeSnapshot({
      connected: false,
      integrationError: message,
      sessions: [],
      instances: [],
    });
    if (message !== lastOpenCodeRefreshError) {
      console.error("Unable to refresh OpenCode sessions", error);
      lastOpenCodeRefreshError = message;
    }
  } finally {
    refreshingOpenCodeSessions = false;
  }
};

sessionDetailBack.addEventListener("click", () => {
  selectedSessionId = undefined;
  selectedCodexSessionId = undefined;
  selectedOpenCodeSessionKey = undefined;
  renderedDetailSignature = undefined;
  switchContentView("sessions");
});

const activeSettingsNavItem = () =>
  settingsNavItems.find(
    (item) => item.getAttribute("aria-selected") === "true",
  ) ?? settingsNavItems[0];

const settingsSectionNames: Record<string, string> = {
  general: "通用",
  hook: "Hook",
  theme: "风格",
  sound: "音效",
  ssh: "SSH",
  lan: "局域网",
  about: "关于",
};
const settingsSectionPlaceholders: Record<string, string> = {};

const formatAnimationSpeed = (speed: number) => `${speed.toFixed(2)}×`;

const syncAnimationSpeedControl = () => {
  const speed = appearanceSettings.animationSpeed;
  const progress =
    ((speed - Number(animationSpeedInput.min)) /
      (Number(animationSpeedInput.max) - Number(animationSpeedInput.min))) *
    100;
  const formattedSpeed = formatAnimationSpeed(speed);

  animationSpeedSetting.hidden = appearanceSettings.motionMode !== "on";
  animationSpeedInput.value = String(speed);
  animationSpeedInput.style.setProperty("--range-progress", `${progress}%`);
  animationSpeedInput.setAttribute("aria-valuetext", formattedSpeed);
  animationSpeedValue.value = formattedSpeed;
  animationSpeedValue.textContent = formattedSpeed;
};

const syncTransparencyControl = (
  input: HTMLInputElement,
  output: HTMLOutputElement,
  transparency: number,
) => {
  const progress =
    ((transparency - Number(input.min)) /
      (Number(input.max) - Number(input.min))) *
    100;
  const formattedTransparency = transparency.toFixed(2);
  input.value = String(transparency);
  input.style.setProperty("--range-progress", `${progress}%`);
  input.setAttribute("aria-valuetext", formattedTransparency);
  output.value = formattedTransparency;
  output.textContent = formattedTransparency;
};

const syncNumericRangeControl = (
  input: HTMLInputElement,
  output: HTMLOutputElement,
  value: number,
  formattedValue: string,
) => {
  const minimum = Number(input.min);
  const maximum = Number(input.max);
  const progress = ((value - minimum) / (maximum - minimum)) * 100;
  input.value = String(value);
  input.style.setProperty("--range-progress", `${progress}%`);
  input.setAttribute("aria-valuetext", formattedValue);
  output.value = formattedValue;
  output.textContent = formattedValue;
};

const setSettingsHeightPanelExpanded = (
  element: HTMLElement,
  expanded: boolean,
) => {
  element.hidden = false;
  element.dataset.expanded = String(expanded);
  element.setAttribute("aria-hidden", String(!expanded));
  element.inert = !expanded;
};

const syncWindowPositionControls = () => {
  const positionPercent = Math.round(
    windowPositionSettings.horizontalPosition * 100,
  );
  topDragEnabledInput.checked = windowPositionSettings.topDragEnabled;
  panelDragRegion.dataset.enabled = String(
    windowPositionSettings.topDragEnabled,
  );
  setSettingsHeightPanelExpanded(
    windowPositionAdvanced,
    windowPositionAdvancedToggle.getAttribute("aria-expanded") === "true",
  );
  syncNumericRangeControl(
    windowPositionInput,
    windowPositionOutput,
    positionPercent,
    `${positionPercent}%`,
  );
  for (const button of windowPositionPresetButtons) {
    const preset = Number(button.dataset.positionPreset);
    button.setAttribute(
      "aria-pressed",
      String(
        Math.abs(preset - windowPositionSettings.horizontalPosition) < 0.005,
      ),
    );
  }
};

let nativePositionFrame: number | undefined;
let nativePositionAnimationPending = false;

// Native generations cancel older animations as the range emits rapid input.
const flushNativeWindowPosition = () => {
  if (!isTauriRuntime) return;
  const requestedPosition = windowPositionSettings.horizontalPosition;
  const animate = nativePositionAnimationPending;
  nativePositionAnimationPending = false;
  void invoke<number>("set_panel_horizontal_position", {
    horizontalPosition: requestedPosition,
    animate,
  }).catch((error: unknown) => {
    console.error("Unable to position the CodeCraft panel", error);
  });
};

const scheduleNativeWindowPosition = (animate = true) => {
  if (!isTauriRuntime) return;
  nativePositionAnimationPending ||= animate;
  if (nativePositionFrame !== undefined) return;
  nativePositionFrame = window.requestAnimationFrame(() => {
    nativePositionFrame = undefined;
    flushNativeWindowPosition();
  });
};

const updateWindowPositionSettings = (
  nextSettings: Partial<WindowPositionSettings>,
  applyPosition = true,
) => {
  windowPositionSettings = {
    ...windowPositionSettings,
    ...nextSettings,
    horizontalPosition: clampHorizontalPosition(
      nextSettings.horizontalPosition ??
        windowPositionSettings.horizontalPosition,
    ),
  };
  persistWindowPositionSettings();
  syncWindowPositionControls();
  if (applyPosition) scheduleNativeWindowPosition();
};

const syncInterfaceControls = () => {
  for (const input of interfaceSizingModeInputs) {
    input.checked = input.value === interfaceSettings.mode;
  }
  setSettingsHeightPanelExpanded(
    interfaceScaleSettings,
    interfaceSettings.mode === "scale",
  );
  setSettingsHeightPanelExpanded(
    interfaceCustomSettings,
    interfaceSettings.mode === "custom",
  );
  syncNumericRangeControl(
    interfaceScaleInput,
    interfaceScaleOutput,
    Math.round(interfaceSettings.scale * 100),
    `${Math.round(interfaceSettings.scale * 100)}%`,
  );
  syncNumericRangeControl(
    interfaceFontScaleInput,
    interfaceFontScaleOutput,
    Math.round(interfaceSettings.customFontScale * 100),
    `${Math.round(interfaceSettings.customFontScale * 100)}%`,
  );
  syncNumericRangeControl(
    interfaceWidthInput,
    interfaceWidthOutput,
    interfaceSettings.customWidth,
    `${interfaceSettings.customWidth}px`,
  );
  syncNumericRangeControl(
    interfaceMinHeightInput,
    interfaceMinHeightOutput,
    interfaceSettings.customMinHeight,
    `${interfaceSettings.customMinHeight}px`,
  );
  syncNumericRangeControl(
    interfaceMaxHeightInput,
    interfaceMaxHeightOutput,
    interfaceSettings.customMaxHeight,
    `${interfaceSettings.customMaxHeight}px`,
  );
  syncNumericRangeControl(
    interfacePaddingInput,
    interfacePaddingOutput,
    interfaceSettings.customPadding,
    `${interfaceSettings.customPadding}px`,
  );
};

let nativeInterfaceLayoutFrame: number | undefined;

const scheduleNativeInterfaceLayout = () => {
  if (nativeInterfaceLayoutFrame !== undefined) return;
  nativeInterfaceLayoutFrame = window.requestAnimationFrame(() => {
    nativeInterfaceLayoutFrame = undefined;
    refreshExpandedPanelHeight();
    syncPanelShape();

    const resize =
      document.documentElement.dataset.panelState === "expanded"
        ? controller.contentResized(true)
        : syncCollapsedPanelSize();
    void resize
      .catch((error: unknown) => {
        console.error("Unable to apply the CodeCraft interface size", error);
      })
      .finally(scheduleNativeWindowPosition);
  });
};

const updateInterfaceSettings = (nextSettings: Partial<InterfaceSettings>) => {
  interfaceSettings = normalizeInterfaceSettings({
    ...interfaceSettings,
    ...nextSettings,
  });
  persistInterfaceSettings();
  applyInterfaceSettings();
  syncInterfaceControls();
  cardSizeAnimator.refresh();
  scheduleNativeInterfaceLayout();
};

const SESSION_CLEANUP_PRESET_LABELS: Record<SessionCleanupPreset, string> = {
  "5m": "5 分钟",
  "10m": "10 分钟",
  "30m": "30 分钟",
  "1h": "1 小时",
  custom: "自定义",
};

const sessionCleanupOptionButtons = () =>
  Array.from(
    sessionCleanupMenu.querySelectorAll<HTMLButtonElement>(
      ".session-cleanup-select__option",
    ),
  );

const moveSessionCleanupChevronToFirst = () => {
  const firstOption = sessionCleanupOptionButtons()[0];
  const firstSlot = firstOption?.querySelector<HTMLElement>(
    ".session-cleanup-select__option-chevron-slot",
  );
  if (!firstSlot) return;

  let chevron = sessionCleanupMenu.querySelector<SVGSVGElement>(
    ".session-cleanup-select__option-chevron",
  );
  if (!chevron) {
    const triggerChevron = sessionCleanupTrigger.querySelector<SVGSVGElement>(
      ".session-cleanup-select__chevron",
    );
    if (triggerChevron) {
      chevron = triggerChevron.cloneNode(true) as SVGSVGElement;
      chevron.classList.add("session-cleanup-select__option-chevron");
    }
  }
  if (chevron) firstSlot.replaceChildren(chevron);
};

const syncSessionCleanupControls = () => {
  const label = SESSION_CLEANUP_PRESET_LABELS[sessionCleanupSettings.preset];
  const selectedOption = sessionCleanupOptionButtons().find(
    (option) => option.dataset.cleanupPreset === sessionCleanupSettings.preset,
  );
  if (selectedOption && sessionCleanupOptionButtons()[0] !== selectedOption) {
    sessionCleanupMenu.prepend(selectedOption);
  }
  moveSessionCleanupChevronToFirst();

  sessionCleanupLabel.textContent = label;
  sessionCleanupTrigger.setAttribute(
    "aria-label",
    `选择清理时间，当前为${label}`,
  );
  for (const option of sessionCleanupOptionButtons()) {
    option.setAttribute(
      "aria-selected",
      String(option.dataset.cleanupPreset === sessionCleanupSettings.preset),
    );
  }
  sessionCleanupCustom.hidden = sessionCleanupSettings.preset !== "custom";
  sessionCleanupCustomMinutesInput.value = String(
    sessionCleanupSettings.customMinutes,
  );
};

const updateSessionCleanupSettings = (
  changes: Partial<SessionCleanupSettings>,
) => {
  sessionCleanupSettings = normalizeSessionCleanupSettings({
    ...sessionCleanupSettings,
    ...changes,
  });
  try {
    window.localStorage.setItem(
      SESSION_CLEANUP_SETTINGS_STORAGE_KEY,
      JSON.stringify(sessionCleanupSettings),
    );
  } catch {
    // The cleanup threshold still applies for the current window.
  }
  syncSessionCleanupControls();
};

const animateSessionCleanupOptionToFirst = async (
  selectedOption: HTMLButtonElement,
) => {
  const options = sessionCleanupOptionButtons();
  const firstOption = options[0];
  if (!firstOption || firstOption === selectedOption) return;

  const previousTops = new Map(
    options.map((option) => [option, option.getBoundingClientRect().top]),
  );
  const chevron = firstOption.querySelector<SVGSVGElement>(
    ".session-cleanup-select__option-chevron",
  );
  const selectedChevronSlot = selectedOption.querySelector<HTMLElement>(
    ".session-cleanup-select__option-chevron-slot",
  );
  if (chevron && selectedChevronSlot) {
    selectedChevronSlot.replaceChildren(chevron);
  }
  sessionCleanupMenu.prepend(selectedOption);

  const animations = sessionCleanupOptionButtons()
    .map((option) => {
      const previousTop = previousTops.get(option);
      if (previousTop === undefined) return undefined;
      const offsetY = previousTop - option.getBoundingClientRect().top;
      if (Math.abs(offsetY) < 0.5) return undefined;
      return option.animate(
        [
          { transform: `translateY(${offsetY}px)` },
          { transform: "translateY(0)" },
        ],
        {
          duration: SESSION_CLEANUP_OPTION_REORDER_MS,
          easing: "cubic-bezier(0.2, 0.7, 0.35, 0.95)",
        },
      );
    })
    .filter((animation): animation is Animation => animation !== undefined);

  await Promise.all(
    animations.map((animation) => animation.finished.catch(() => undefined)),
  );
};

const closeSessionCleanupMenu = (restoreFocus = false) => {
  if (sessionCleanupMenuCloseTimer !== undefined) {
    clearTimeout(sessionCleanupMenuCloseTimer);
    sessionCleanupMenuCloseTimer = undefined;
  }
  if (sessionCleanupMenuOpenFrame !== undefined) {
    cancelAnimationFrame(sessionCleanupMenuOpenFrame);
    sessionCleanupMenuOpenFrame = undefined;
  }
  if (sessionCleanupMenu.hidden) {
    if (restoreFocus) sessionCleanupTrigger.focus({ preventScroll: true });
    return;
  }

  sessionCleanupTrigger.setAttribute("aria-expanded", "false");
  sessionCleanupMenu.dataset.open = "false";
  sessionCleanupMenuCloseTimer = setTimeout(() => {
    sessionCleanupMenu.hidden = true;
    sessionCleanupMenuCloseTimer = undefined;
  }, SESSION_CLEANUP_MENU_TRANSITION_MS + SESSION_CLEANUP_MENU_FADE_MS);
  if (restoreFocus) sessionCleanupTrigger.focus({ preventScroll: true });
};

const openSessionCleanupMenu = (focusIndex = 0) => {
  if (sessionCleanupMenuCloseTimer !== undefined) {
    clearTimeout(sessionCleanupMenuCloseTimer);
    sessionCleanupMenuCloseTimer = undefined;
  }
  if (sessionCleanupMenuOpenFrame !== undefined) {
    cancelAnimationFrame(sessionCleanupMenuOpenFrame);
  }
  syncSessionCleanupControls();
  sessionCleanupMenu.dataset.open = "false";
  sessionCleanupMenu.hidden = false;
  sessionCleanupMenuOpenFrame = requestAnimationFrame(() => {
    sessionCleanupMenuOpenFrame = undefined;
    sessionCleanupTrigger.setAttribute("aria-expanded", "true");
    sessionCleanupMenu.dataset.open = "true";
  });
  const options = sessionCleanupOptionButtons();
  options[Math.max(0, Math.min(focusIndex, options.length - 1))]?.focus({
    preventScroll: true,
  });
};

const selectSessionCleanupOption = async (option: HTMLButtonElement) => {
  if (sessionCleanupSelectionPending) return;
  const preset = option.dataset.cleanupPreset as
    SessionCleanupPreset | undefined;
  if (!preset) return;

  sessionCleanupSelectionPending = true;
  sessionCleanupMenu.dataset.reordering = "true";
  sessionCleanupMenu.setAttribute("aria-busy", "true");
  try {
    await animateSessionCleanupOptionToFirst(option);
    updateSessionCleanupSettings({ preset });
    closeSessionCleanupMenu(true);
  } finally {
    sessionCleanupSelectionPending = false;
    delete sessionCleanupMenu.dataset.reordering;
    sessionCleanupMenu.removeAttribute("aria-busy");
  }
};

sessionCleanupTrigger.addEventListener("click", () => {
  if (sessionCleanupTrigger.getAttribute("aria-expanded") === "true") {
    closeSessionCleanupMenu();
  } else {
    openSessionCleanupMenu();
  }
});

sessionCleanupTrigger.addEventListener("keydown", (event) => {
  if (!["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) return;
  event.preventDefault();
  openSessionCleanupMenu(
    event.key === "ArrowUp" ? sessionCleanupPresetOptions.length - 1 : 0,
  );
});

for (const option of sessionCleanupPresetOptions) {
  option.addEventListener("click", () => {
    void selectSessionCleanupOption(option);
  });
}

sessionCleanupMenu.addEventListener("keydown", (event) => {
  if (sessionCleanupSelectionPending) {
    event.preventDefault();
    return;
  }
  if (event.key === "Enter" || event.key === " ") {
    const option =
      event.target instanceof Element
        ? event.target.closest<HTMLButtonElement>("[data-cleanup-preset]")
        : null;
    if (option) {
      event.preventDefault();
      void selectSessionCleanupOption(option);
    }
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    closeSessionCleanupMenu(true);
    return;
  }
  if (event.key === "Tab") {
    closeSessionCleanupMenu();
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;

  event.preventDefault();
  const options = sessionCleanupOptionButtons();
  const currentIndex = options.indexOf(
    document.activeElement as HTMLButtonElement,
  );
  const nextIndex =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? options.length - 1
        : event.key === "ArrowDown"
          ? (currentIndex + 1) % options.length
          : (currentIndex - 1 + options.length) % options.length;
  options[nextIndex]?.focus({ preventScroll: true });
});

document.addEventListener("pointerdown", (event) => {
  if (sessionCleanupTrigger.getAttribute("aria-expanded") !== "true") return;
  if (sessionCleanupSelectionPending) return;
  if (
    event.target instanceof Node &&
    !sessionCleanupSelect.contains(event.target)
  ) {
    closeSessionCleanupMenu();
  }
});

sessionCleanupCustomMinutesInput.addEventListener("input", () => {
  if (!sessionCleanupCustomMinutesInput.validity.valid) return;
  updateSessionCleanupSettings({
    customMinutes: sessionCleanupCustomMinutesInput.valueAsNumber,
  });
});

sessionCleanupCustomMinutesInput.addEventListener("change", () => {
  if (!sessionCleanupCustomMinutesInput.validity.valid) {
    syncSessionCleanupControls();
  }
});

syncSessionCleanupControls();

type ApprovalMode = "manual" | "risk" | "automatic";
type ApprovalSettings = { mode: ApprovalMode };
const APPROVAL_SETTINGS_STORAGE_KEY = "codecraft.approval-settings";
let approvalSettings: ApprovalSettings = { mode: "manual" };

const loadApprovalSettings = () => {
  try {
    const stored = JSON.parse(
      window.localStorage.getItem(APPROVAL_SETTINGS_STORAGE_KEY) ?? "null",
    ) as { mode?: unknown } | null;
    if (stored?.mode === "permission") {
      return { mode: "risk" } satisfies ApprovalSettings;
    }
    if (
      stored?.mode === "manual" ||
      stored?.mode === "risk" ||
      stored?.mode === "automatic"
    ) {
      return { mode: stored.mode } satisfies ApprovalSettings;
    }
  } catch {
    // Use the safe default when browser storage is unavailable or malformed.
  }
  return { mode: "manual" as const };
};

const syncApprovalControls = () => {
  for (const input of approvalModeInputs) {
    input.checked = input.value === approvalSettings.mode;
  }
};

const refreshApprovalSettings = async () => {
  approvalSettings = loadApprovalSettings();
  if (isTauriRuntime) {
    try {
      approvalSettings = await invoke<ApprovalSettings>(
        "get_approval_settings",
      );
    } catch {
      // The browser fallback keeps the control usable during preview builds.
    }
  }
  syncApprovalControls();
};

const updateApprovalSettings = async (mode: ApprovalMode) => {
  approvalSettings = { mode };
  try {
    window.localStorage.setItem(
      APPROVAL_SETTINGS_STORAGE_KEY,
      JSON.stringify(approvalSettings),
    );
  } catch {
    // The native command remains the source of truth in the desktop app.
  }
  if (isTauriRuntime) {
    try {
      approvalSettings = await invoke<ApprovalSettings>(
        "set_approval_settings",
        {
          mode,
        },
      );
    } catch (error) {
      console.error("Unable to save approval settings", error);
    }
  }
  syncApprovalControls();
};

for (const input of approvalModeInputs) {
  input.addEventListener("change", () => {
    if (input.checked) void updateApprovalSettings(input.value as ApprovalMode);
  });
}

const formatAutoCollapseDelay = (seconds: number) => `${seconds.toFixed(1)}s`;

const syncCollapseExpandControls = () => {
  const delay = collapseExpandSettings.autoCollapseDelaySeconds;
  const minimum = Number(autoCollapseDelayInput.min);
  const maximum = Number(autoCollapseDelayInput.max);
  const progress = ((delay - minimum) / (maximum - minimum)) * 100;
  const formattedDelay = formatAutoCollapseDelay(delay);

  autoCollapseDelayInput.value = String(delay);
  autoCollapseDelayInput.style.setProperty("--range-progress", `${progress}%`);
  autoCollapseDelayInput.setAttribute("aria-valuetext", formattedDelay);
  autoCollapseDelayOutput.value = formattedDelay;
  autoCollapseDelayOutput.textContent = formattedDelay;
  approvalAutoExpandInput.checked = collapseExpandSettings.approvalAutoExpand;
};

const updateCollapseExpandSettings = (
  changes: Partial<CollapseExpandSettings>,
) => {
  collapseExpandSettings = {
    ...collapseExpandSettings,
    ...changes,
  };
  collapseExpandSettings.autoCollapseDelaySeconds = clampAutoCollapseDelay(
    collapseExpandSettings.autoCollapseDelaySeconds,
  );
  controller.setCollapseDelay(autoCollapseDelayMs(collapseExpandSettings));
  try {
    window.localStorage.setItem(
      COLLAPSE_EXPAND_SETTINGS_STORAGE_KEY,
      JSON.stringify(collapseExpandSettings),
    );
  } catch {
    // The settings still apply for the current window.
  }
  syncCollapseExpandControls();
};

autoCollapseDelayInput.addEventListener("input", () => {
  updateCollapseExpandSettings({
    autoCollapseDelaySeconds: Number(autoCollapseDelayInput.value),
  });
});

approvalAutoExpandInput.addEventListener("change", () => {
  updateCollapseExpandSettings({
    approvalAutoExpand: approvalAutoExpandInput.checked,
  });
});

syncCollapseExpandControls();

type HookAgentId = "claudeCode" | "codex" | "openCode";
type HookIntegrationStatus = {
  id: HookAgentId;
  name: string;
  agentInstalled: boolean;
  hookInstalled: boolean;
  installState?:
    | "notInstalled"
    | "installed"
    | "syncedRestartRequired"
    | "modified"
    | "conflict"
    | "incompatible"
    | "error";
  installPath?: string;
  bundledVersion?: string;
  installedVersion?: string;
  runningVersions?: string[];
  error?: string;
};
type HookOperation = { id: HookAgentId; installing: boolean };

const hookAgentButtons = new Map<HookAgentId, HTMLButtonElement>();
let hookIntegrations: HookIntegrationStatus[] = [];
let hookOperation: HookOperation | undefined;
let hookRefreshing = false;
let hookRefreshLabelTarget = hookRefreshLabel.textContent ?? "刷新";
let hookRefreshLabelTransitionToken = 0;
let hookRefreshLabelAnimation: Animation | undefined;

const browserHookIntegrations: HookIntegrationStatus[] = [
  {
    id: "claudeCode",
    name: "Claude Code",
    agentInstalled: true,
    hookInstalled: true,
  },
  { id: "codex", name: "Codex", agentInstalled: true, hookInstalled: false },
  {
    id: "openCode",
    name: "OpenCode",
    agentInstalled: true,
    hookInstalled: false,
  },
];

const openCodeHookIntegration = () =>
  hookIntegrations.find((status) => status.id === "openCode");

const codexHookIntegration = () =>
  hookIntegrations.find((status) => status.id === "codex");

const isCodexHookKnownUninstalled = () => {
  const status = codexHookIntegration();
  return status !== undefined && !status.hookInstalled;
};

const isOpenCodeHookHealthy = () => {
  const status = openCodeHookIntegration();
  return Boolean(status?.hookInstalled && !status.error);
};

const syncOpenCodeHookStatus = () => {
  const status = openCodeHookIntegration();
  const healthy = isOpenCodeHookHealthy();
  const restartRequired =
    status?.installState === "syncedRestartRequired" ||
    latestOpenCodeSnapshot.instances.some((instance) => instance.restartRequired);
  setSourceStatusLabel(
    openCodeConnectionStatus,
    restartRequired ? "需重启 OpenCode" : healthy ? "Hook 正常" : "Hook 异常",
  );
  openCodeConnectionStatus.dataset.connected = String(healthy && !restartRequired);
  openCodeConnectionStatus.title = restartRequired
    ? "请重启 OpenCode 以加载新版 Hook"
    : healthy
    ? "OpenCode Hook 正常；启动 OpenCode 后即可同步会话"
    : (status?.error ??
      latestOpenCodeSnapshot.integrationError ??
      "OpenCode Hook 未安装");
};

const setHookSettingsError = (message?: string) => {
  hookSettingsStatus.hidden = !message;
  hookSettingsStatus.textContent = message ?? "";
};

const transitionHookRefreshLabel = (nextLabel: string) => {
  if (nextLabel === hookRefreshLabelTarget) return;
  hookRefreshLabelTarget = nextLabel;
  const transitionToken = ++hookRefreshLabelTransitionToken;
  hookRefreshLabelAnimation?.cancel();

  const outgoing = hookRefreshLabel.animate(
    [
      { opacity: 1, transform: "translateY(0)" },
      { opacity: 0, transform: "translateY(-3px)" },
    ],
    { duration: 90, easing: "ease-in", fill: "both" },
  );
  hookRefreshLabelAnimation = outgoing;
  void outgoing.finished
    .then(() => {
      if (transitionToken !== hookRefreshLabelTransitionToken) return;
      hookRefreshLabel.textContent = nextLabel;
      const incoming = hookRefreshLabel.animate(
        [
          { opacity: 0, transform: "translateY(3px)" },
          { opacity: 1, transform: "translateY(0)" },
        ],
        {
          duration: 140,
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          fill: "both",
        },
      );
      hookRefreshLabelAnimation = incoming;
      return incoming.finished;
    })
    .then(() => {
      if (transitionToken !== hookRefreshLabelTransitionToken) return;
      hookRefreshLabelAnimation = undefined;
      hookRefreshLabel.style.removeProperty("opacity");
      hookRefreshLabel.style.removeProperty("transform");
    })
    .catch(() => {
      // A newer refresh state owns the label transition.
    });
};

const syncHookRefreshButton = () => {
  const loading = hookRefreshing || hookOperation !== undefined;
  hookRefreshButton.disabled = loading;
  hookRefreshButton.dataset.loading = String(loading);
  transitionHookRefreshLabel(
    hookRefreshing ? "刷新中..." : hookOperation ? "处理中..." : "刷新",
  );
};

const hookIconUrl = (id: HookAgentId) => {
  if (id === "claudeCode") return claudeCodeIconUrl;
  if (id === "openCode") return openCodeIconUrl;
  return codexIconUrl;
};

const createHookAgentButton = (status: HookIntegrationStatus) => {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "hook-agent-button";

  const mark = document.createElement("span");
  mark.className = "hook-agent-button__mark";
  mark.dataset.agent =
    status.id === "claudeCode"
      ? "claude"
      : status.id === "openCode"
        ? "opencode"
        : "codex";
  mark.setAttribute("aria-hidden", "true");
  const image = document.createElement("img");
  image.alt = "";
  image.src = hookIconUrl(status.id);
  mark.append(image);

  const copy = document.createElement("span");
  copy.className = "hook-agent-button__copy";
  const name = document.createElement("strong");
  name.textContent = status.name;
  const detail = document.createElement("small");
  detail.textContent =
    status.id === "openCode" && status.installPath
      ? status.installPath
      : "CodeCraft 会话同步与审阅 Hook";
  copy.append(name, detail);

  const state = document.createElement("span");
  state.className = "hook-agent-button__state";
  button.append(mark, copy, state);
  button.addEventListener("click", () => void toggleAgentHook(status.id));
  hookAgentButtons.set(status.id, button);
  return button;
};

const syncHookAgentButton = (
  button: HTMLButtonElement,
  status: HookIntegrationStatus,
) => {
  const state = button.querySelector<HTMLElement>(".hook-agent-button__state")!;
  const activeOperation =
    hookOperation?.id === status.id ? hookOperation : undefined;
  const detail = button.querySelector<HTMLElement>(
    ".hook-agent-button__copy small",
  );
  if (detail) {
    detail.textContent =
      status.id === "openCode" && status.installPath
        ? status.installPath
        : "CodeCraft 会话同步与审阅 Hook";
  }
  button.dataset.agentInstalled = String(status.agentInstalled);
  button.dataset.busy = String(activeOperation !== undefined);
  button.dataset.refreshing = String(hookRefreshing);
  button.disabled =
    hookRefreshing || hookOperation !== undefined || !status.agentInstalled;
  button.setAttribute(
    "aria-label",
    !status.agentInstalled
      ? `${status.name} 未安装`
      : `${status.hookInstalled ? "卸载" : "安装"} ${status.name} Hook`,
  );
  button.title = !status.agentInstalled
    ? `未检测到 ${status.name}，无法操作 Hook`
    : status.error
      ? status.error
      : `点击${status.hookInstalled ? "卸载" : "安装"} ${status.name} Hook`;

  state.replaceChildren();
  if (activeOperation) {
    state.dataset.kind = "operation";
    const text = document.createElement("span");
    text.textContent = activeOperation.installing
      ? "安装Hook中..."
      : "卸载Hook中...";
    const spinner = document.createElement("span");
    spinner.className = "hook-spinner";
    spinner.setAttribute("aria-hidden", "true");
    state.append(text, spinner);
  } else if (!status.agentInstalled) {
    state.dataset.kind = "missing";
    state.textContent = "未安装";
  } else if (status.installState === "syncedRestartRequired") {
    state.dataset.kind = "action";
    state.textContent = "已同步 · 重启生效";
  } else if (
    status.id === "openCode" &&
    latestOpenCodeSnapshot.instances.some((instance) => instance.restartRequired)
  ) {
    state.dataset.kind = "action";
    state.textContent = "请重启 OpenCode";
  } else if (status.error) {
    state.dataset.kind = "missing";
    state.textContent = "需处理";
  } else {
    state.dataset.kind = "action";
    state.textContent = status.hookInstalled ? "卸载" : "安装";
  }
};

const animateHookLayout = (firstRects: Map<HookAgentId, DOMRect>) => {
  for (const [id, button] of hookAgentButtons) {
    const first = firstRects.get(id);
    if (!first || !button.isConnected) continue;
    const last = button.getBoundingClientRect();
    const x = first.left - last.left;
    const y = first.top - last.top;
    if (Math.abs(x) < 0.5 && Math.abs(y) < 0.5) continue;
    button.animate(
      [
        { transform: `translate3d(${x}px, ${y}px, 0)`, zIndex: 2 },
        { transform: "translate3d(0, 0, 0)", zIndex: 2 },
      ],
      { duration: 320, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
    );
  }
};

const renderHookIntegrations = (
  nextStatuses: HookIntegrationStatus[],
  animate = true,
) => {
  const firstRects = new Map<HookAgentId, DOMRect>();
  if (animate && !hookSettings.hidden) {
    for (const [id, button] of hookAgentButtons) {
      if (button.isConnected)
        firstRects.set(id, button.getBoundingClientRect());
    }
  }

  hookIntegrations = nextStatuses;
  syncInstalledSessionProducts(nextStatuses);
  syncOpenCodeHookStatus();
  if (isTauriRuntime && isCodexHookKnownUninstalled()) {
    // Removing a Hook must clear any already-rendered Codex detail or pending
    // request immediately; waiting for the next session poll leaves stale
    // agent content visible for one refresh interval.
    renderCodexSnapshot({
      ...latestCodexSnapshot,
      connected: false,
      integrationError: latestCodexSnapshot.integrationError ?? "Codex Hook 未安装",
      sessions: [],
      interactions: [],
    });
  }
  const installed = nextStatuses.filter((status) => status.hookInstalled);
  const available = nextStatuses.filter((status) => !status.hookInstalled);
  for (const status of nextStatuses) {
    const button =
      hookAgentButtons.get(status.id) ?? createHookAgentButton(status);
    syncHookAgentButton(button, status);
    (status.hookInstalled ? installedHookList : availableHookList).append(
      button,
    );
  }
  installedHookCount.textContent = String(installed.length);
  availableHookCount.textContent = String(available.length);
  installedHookEmpty.hidden = installed.length > 0;
  availableHookEmpty.hidden = available.length > 0;
  syncHookRefreshButton();

  if (animate) {
    window.requestAnimationFrame(() => animateHookLayout(firstRects));
  }
};

const readHookIntegrations = () =>
  isTauriRuntime
    ? invoke<HookIntegrationStatus[]>("list_hook_integrations")
    : Promise.resolve(browserHookIntegrations.map((status) => ({ ...status })));

const refreshHookIntegrations = async () => {
  if (hookRefreshing || hookOperation) return;
  hookRefreshing = true;
  setHookSettingsError();
  renderHookIntegrations(hookIntegrations, false);
  try {
    const statuses = await readHookIntegrations();
    hookRefreshing = false;
    renderHookIntegrations(statuses);
  } catch (error) {
    hookRefreshing = false;
    renderHookIntegrations(hookIntegrations, false);
    setHookSettingsError(`刷新 Hook 状态失败：${String(error)}`);
  }
};

const fadeHookOperation = async (id: HookAgentId) => {
  const state = hookAgentButtons
    .get(id)
    ?.querySelector<HTMLElement>(
      '.hook-agent-button__state[data-kind="operation"]',
    );
  if (!state) return;
  try {
    await state.animate(
      [
        { opacity: 1, transform: "translateX(0)" },
        { opacity: 0, transform: "translateX(7px)" },
      ],
      { duration: 140, easing: "ease-in" },
    ).finished;
  } catch {
    // A newer render owns the state.
  }
};

async function toggleAgentHook(id: HookAgentId) {
  const status = hookIntegrations.find((item) => item.id === id);
  if (!status || !status.agentInstalled || hookOperation || hookRefreshing)
    return;
  hookOperation = { id, installing: !status.hookInstalled };
  setHookSettingsError();
  renderHookIntegrations(hookIntegrations, false);
  try {
    if (isTauriRuntime) {
      await invoke(
        status.hookInstalled ? "uninstall_agent_hook" : "install_agent_hook",
        {
          agent: id,
        },
      );
    } else {
      await new Promise((resolve) => window.setTimeout(resolve, 650));
      const preview = browserHookIntegrations.find((item) => item.id === id);
      if (preview) preview.hookInstalled = !status.hookInstalled;
    }
    const nextStatuses = await readHookIntegrations();
    await fadeHookOperation(id);
    hookOperation = undefined;
    renderHookIntegrations(nextStatuses);
  } catch (error) {
    await fadeHookOperation(id);
    hookOperation = undefined;
    renderHookIntegrations(hookIntegrations, false);
    setHookSettingsError(
      `${status.hookInstalled ? "卸载" : "安装"} Hook 失败：${String(error)}`,
    );
  }
}

hookRefreshButton.addEventListener(
  "click",
  () => void refreshHookIntegrations(),
);

const LAN_STATUS_INTERVAL_MS = 1500;

let lanConfig: LanServerConfigView = defaultLanConfig();
let lanStatus: LanStatusView = lanStatusFor(lanConfig);
let lanTokenRevealed = false;
let lanEnableArmed = false;
let lanRotateArmed = false;
let lanBusy = false;
let lanAdvancedExpanded = false;
let lanQrUrl: string | undefined;
let lanStatusTimer: ReturnType<typeof setInterval> | undefined;
const lanAddressRows = new Map<string, HTMLElement>();

// Preview builds have no Tauri bridge, so the panel edits an in-memory copy.
const browserLanConfig: LanServerConfigView = {
  ...defaultLanConfig(),
  token: "0123456789abcdef0123456789abcdef",
};

/** Rotates the preview token without leaving the browser fallback. */
const previewLanToken = () => {
  const alphabet = "0123456789abcdef";
  let token = "";
  while (token.length < browserLanConfig.token.length) {
    token += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  browserLanConfig.token = token;
  return token;
};

const loadLanAcknowledged = () => {
  try {
    return window.localStorage.getItem(LAN_ACKNOWLEDGED_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
};

let lanAcknowledged = loadLanAcknowledged();

const rememberLanAcknowledged = () => {
  if (lanAcknowledged) return;
  lanAcknowledged = true;
  try {
    window.localStorage.setItem(LAN_ACKNOWLEDGED_STORAGE_KEY, "1");
  } catch {
    // The confirmation still applies to this session.
  }
};

type LanNoticeTone = "error" | "warn" | "hint";

const LAN_CARD_HEIGHT_TRANSITION_MS = 220;
const LAN_CARD_HEIGHT_EASING = "cubic-bezier(0.22, 1, 0.36, 1)";

interface LanCardHeightAnimation {
  animation: Animation;
  hideOnFinish: boolean;
}

const lanCardAnimations = new Map<HTMLElement, LanCardHeightAnimation>();
/** Whether a card should end up hidden, even while it is still collapsing. */
const lanCardHiddenIntent = new WeakMap<HTMLElement, boolean>();
let lanCardTransitionDepth = 0;

const lanCardElements = () =>
  Array.from(lanSettings.querySelectorAll<HTMLElement>(".settings-card"));

/**
 * A card in flight measures at its current animated height, which is exactly
 * what an interrupted transition should continue from.
 */
const measureLanCard = (card: HTMLElement): LanCardHeightSnapshot => ({
  height: card.getBoundingClientRect().height,
  hidden: lanCardHiddenIntent.get(card) ?? card.hidden,
});

/**
 * Hiding a card would remove it from the layout before it can shrink, so a
 * collapsing card stays visible and the animation hides it at the end.
 */
const setLanCardHidden = (card: HTMLElement, hidden: boolean) => {
  lanCardHiddenIntent.set(card, hidden);
  if (hidden && lanCardAnimations.get(card)?.hideOnFinish) return;
  card.hidden = hidden;
};

/** Releases a running animation so the card can be measured at rest. */
const releaseLanCardAnimation = (card: HTMLElement) => {
  const active = lanCardAnimations.get(card);
  if (!active) return;
  lanCardAnimations.delete(card);
  active.animation.cancel();
  card.style.removeProperty("height");
  card.style.removeProperty("overflow");
  card.style.removeProperty("opacity");
};

const playLanCardHeight = (card: HTMLElement, plan: LanCardHeightPlan) => {
  if (plan.kind === "none") return;

  const hideOnFinish = plan.kind === "collapse";
  const frames =
    plan.kind === "collapse"
      ? [
          { height: `${plan.from}px`, opacity: 1 },
          { height: "0px", opacity: 0 },
        ]
      : plan.kind === "reveal"
        ? [
            { height: `${plan.from}px`, opacity: 0 },
            { height: `${plan.to}px`, opacity: 1 },
          ]
        : [{ height: `${plan.from}px` }, { height: `${plan.to}px` }];

  if (hideOnFinish) card.hidden = false;
  card.style.overflow = "hidden";
  const animation = card.animate(frames, {
    duration: LAN_CARD_HEIGHT_TRANSITION_MS,
    easing: LAN_CARD_HEIGHT_EASING,
    fill: "both",
  });
  const record: LanCardHeightAnimation = { animation, hideOnFinish };
  lanCardAnimations.set(card, record);

  const settle = () => {
    if (lanCardAnimations.get(card) !== record) return;
    lanCardAnimations.delete(card);
    if (hideOnFinish) card.hidden = true;
    animation.cancel();
    card.style.removeProperty("height");
    card.style.removeProperty("overflow");
    card.style.removeProperty("opacity");
  };

  // Motion can be turned off, which finishes the animation before a listener
  // could attach, so the settled promise covers both paths.
  void animation.finished.then(settle, settle);
};

/**
 * Runs a DOM update and turns every resulting card height change into an
 * animation. Nested calls belong to the outermost update.
 */
const animateLanCardHeights = (mutate: () => void) => {
  if (lanCardTransitionDepth > 0 || lanSettings.hidden || settingsView.hidden) {
    mutate();
    return;
  }

  const previous = new Map(
    lanCardElements().map((card) => [card, measureLanCard(card)] as const),
  );

  lanCardTransitionDepth += 1;
  try {
    mutate();
  } finally {
    lanCardTransitionDepth -= 1;
  }

  for (const card of lanCardElements()) {
    const before = previous.get(card);
    if (!before) continue;

    const hidden = lanCardHiddenIntent.get(card) ?? card.hidden;
    // Already hidden, or still collapsing towards hidden.
    if (hidden && before.hidden) continue;

    releaseLanCardAnimation(card);
    const after: LanCardHeightSnapshot = {
      height: hidden ? 0 : card.getBoundingClientRect().height,
      hidden,
    };
    playLanCardHeight(card, planLanCardHeight(before, after));
  }
};

const setLanNotice = (message?: string, tone: LanNoticeTone = "error") => {
  animateLanCardHeights(() => {
    lanError.hidden = !message;
    lanError.textContent = message ?? "";
    lanError.dataset.tone = tone;
  });
};

const readLanConfig = () =>
  isTauriRuntime
    ? invoke<LanServerConfigView>("lan_get_config")
    : Promise.resolve({ ...browserLanConfig });

const readLanStatus = () =>
  isTauriRuntime
    ? invoke<LanStatusView>("lan_status")
    : Promise.resolve(lanStatusFor(browserLanConfig));

/** Only the fields lan_set_config accepts; the token has its own command. */
type LanConfigChanges = Partial<Omit<LanServerConfigView, "token">>;

const writeLanConfig = (changes: LanConfigChanges) => {
  if (isTauriRuntime) return invoke<LanStatusView>("lan_set_config", changes);
  Object.assign(browserLanConfig, changes);
  return Promise.resolve(lanStatusFor(browserLanConfig));
};

const copyLanText = async (value: string) => {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
};

const flashLanAction = (element: HTMLElement, message: string) => {
  const previous = element.dataset.restoreLabel ?? element.textContent ?? "";
  element.dataset.restoreLabel = previous;
  element.textContent = message;
  element.animate(
    [
      { opacity: 0, transform: "translateY(2px)" },
      { opacity: 1, transform: "translateY(0)" },
    ],
    { duration: 140, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
  );
  window.setTimeout(() => {
    element.textContent = element.dataset.restoreLabel ?? previous;
    delete element.dataset.restoreLabel;
  }, 1200);
};

const hideLanQrCode = () => {
  animateLanCardHeights(() => {
    lanQrUrl = undefined;
    lanQr.hidden = true;
    lanQr.replaceChildren();
  });
};

const toggleLanQrCode = async (url: string) => {
  if (lanQrUrl === url) {
    hideLanQrCode();
    return;
  }
  if (!isTauriRuntime) {
    animateLanCardHeights(() => {
      lanQrUrl = url;
      lanQr.hidden = false;
      lanQr.textContent = "预览模式下不生成二维码";
    });
    return;
  }
  try {
    const svg = await invoke<string>("lan_address_qr_code", { url });
    animateLanCardHeights(() => {
      lanQrUrl = url;
      lanQr.innerHTML = svg;
      lanQr.hidden = false;
    });
  } catch (error) {
    hideLanQrCode();
    setLanNotice("生成二维码失败：" + String(error));
  }
};

const createLanAddressRow = (url: string) => {
  const row = document.createElement("div");
  row.className = "lan-field lan-address-row";

  const copyButton = document.createElement("button");
  copyButton.type = "button";
  copyButton.className = "lan-address";
  copyButton.title = "点击复制 " + url;

  const urlText = document.createElement("span");
  urlText.className = "lan-address__url";
  urlText.textContent = url;

  const actionText = document.createElement("span");
  actionText.className = "lan-address__action";
  actionText.textContent = "复制";

  copyButton.append(urlText, actionText);
  copyButton.addEventListener("click", () => {
    void copyLanText(url).then((copied) => {
      flashLanAction(actionText, copied ? "已复制" : "复制失败");
    });
  });

  const qrButton = document.createElement("button");
  qrButton.type = "button";
  qrButton.className = "lan-button";
  qrButton.textContent = "二维码";
  qrButton.addEventListener("click", () => void toggleLanQrCode(url));

  row.append(copyButton, qrButton);
  return row;
};

const renderLanAddresses = () => {
  const addresses = lanStatus.addresses;
  const seen = new Set<string>();
  let index = 0;

  for (const address of addresses) {
    seen.add(address.url);
    let row = lanAddressRows.get(address.url);
    if (!row) {
      row = createLanAddressRow(address.url);
      lanAddressRows.set(address.url, row);
      lanAddressList.append(row);
      row.animate(
        [
          { opacity: 0, transform: "translateY(6px)" },
          { opacity: 1, transform: "translateY(0)" },
        ],
        {
          duration: 180,
          delay: index * 24,
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          fill: "both",
        },
      );
    } else {
      lanAddressList.append(row);
    }
    index += 1;
  }

  for (const [url, row] of lanAddressRows) {
    if (seen.has(url)) continue;
    lanAddressRows.delete(url);
    row.remove();
    if (lanQrUrl === url) hideLanQrCode();
  }

  setLanCardHidden(lanAddressCard, !lanStatus.running);
  lanAddressEmpty.hidden = addresses.length > 0;
  if (!lanStatus.running) hideLanQrCode();
};

const applyLanSettings = () => {
  const descriptor = lanStateDescriptor(lanStatus);
  lanState.dataset.state = descriptor.state;
  lanState.textContent = descriptor.label;

  lanEnabledInput.checked = lanStatus.running || lanConfig.enabled;
  lanAllowApprovalsInput.checked = lanConfig.allowApprovals;
  lanAuditRemoteInput.checked = lanConfig.auditRemote;
  // Reading the token stays available while the server runs, since that is
  // exactly when someone needs it to sign in from a browser.
  const locked = lanAdvancedLocked(lanStatus);
  // A pending rotation confirmation cannot survive the server coming up.
  if (locked) lanRotateArmed = false;

  if (locked || document.activeElement !== lanPortInput) {
    lanPortInput.value = String(lanConfig.port);
  }

  lanTokenText.textContent = lanTokenRevealed
    ? lanConfig.token || "尚未生成"
    : maskLanToken(lanConfig.token);
  lanTokenRevealButton.textContent = lanTokenRevealed ? "隐藏" : "显示";
  lanTokenRevealButton.setAttribute("aria-pressed", String(lanTokenRevealed));
  lanTokenRotateButton.textContent = lanRotateArmed ? "确认轮换" : "轮换";

  // Client activity only means something once the server is listening.
  lanClients.hidden = !lanStatus.running;
  lanClientCount.textContent = String(lanStatus.clientCount);
  lanLastClient.textContent = lastClientLabel(
    lanStatus.lastClientAt,
    Date.now(),
  );

  lanAdvancedToggle.setAttribute("aria-expanded", String(lanAdvancedExpanded));
  lanAdvanced.hidden = !lanAdvancedExpanded;
  lanAdvancedLock.hidden = !locked;
  lanAdvancedLock.textContent = locked ? LAN_ADVANCED_LOCK_HINT : "";

  lanEnabledInput.disabled = lanBusy;
  lanAllowApprovalsInput.disabled = lanBusy;
  lanAuditRemoteInput.disabled = lanBusy;
  lanPortInput.disabled = lanBusy || locked;
  lanPortApplyButton.disabled = lanBusy || locked;
  lanTokenRotateButton.disabled = lanBusy || locked;
  lanTokenCopyButton.disabled = lanBusy || !lanConfig.token;
  lanTokenRevealButton.disabled = lanBusy || !lanConfig.token;

  // While the server runs and nothing else needs saying, the notice line
  // states whether the browser can act or only watch.
  if (
    lanStatus.running &&
    (lanError.hidden || lanError.dataset.tone === "hint")
  ) {
    setLanNotice(lanReadOnlyHint(lanStatus), "hint");
  } else if (!lanStatus.running && lanError.dataset.tone === "hint") {
    setLanNotice();
  }

  renderLanAddresses();
};

/** Every render can change a card's height, so all of them are animated. */
const renderLanSettings = () => {
  animateLanCardHeights(applyLanSettings);
};

const refreshLanStatus = async () => {
  try {
    const [config, status] = await Promise.all([
      readLanConfig(),
      readLanStatus(),
    ]);
    lanConfig = config;
    lanStatus = status;
    if (status.lastError && !status.running) {
      setLanNotice(status.lastError);
    }
    renderLanSettings();
  } catch (error) {
    setLanNotice("读取局域网状态失败：" + String(error));
  }
};

const applyLanConfig = async (changes: LanConfigChanges) => {
  if (lanBusy) return;
  lanBusy = true;
  setLanNotice();
  renderLanSettings();
  try {
    lanStatus = await writeLanConfig(changes);
    lanConfig = await readLanConfig();
    if (changes.enabled === true) rememberLanAcknowledged();
    if (changes.allowApprovals === true)
      setLanNotice(APPROVALS_RISK_HINT, "warn");
  } catch (error) {
    setLanNotice(String(error));
    await refreshLanStatus().catch(() => undefined);
  } finally {
    lanBusy = false;
    renderLanSettings();
  }
};

const startLanStatusPolling = () => {
  if (lanStatusTimer !== undefined) return;
  lanStatusTimer = setInterval(() => {
    // The panel keeps the section selected after the user leaves settings, so
    // visibility decides whether polling is still worth it.
    if (lanBusy || lanSettings.hidden || settingsView.hidden) return;
    void refreshLanStatus();
  }, LAN_STATUS_INTERVAL_MS);
};

const stopLanStatusPolling = () => {
  if (lanStatusTimer === undefined) return;
  clearInterval(lanStatusTimer);
  lanStatusTimer = undefined;
};

lanEnabledInput.addEventListener("change", () => {
  const enabled = lanEnabledInput.checked;
  if (enabled && needsEnableConfirmation(lanAcknowledged, lanEnableArmed)) {
    lanEnableArmed = true;
    lanEnabledInput.checked = false;
    setLanNotice(ENABLE_CONFIRM_HINT, "warn");
    return;
  }
  lanEnableArmed = false;
  void applyLanConfig({ enabled });
});

lanAllowApprovalsInput.addEventListener("change", () => {
  void applyLanConfig({ allowApprovals: lanAllowApprovalsInput.checked });
});

lanAuditRemoteInput.addEventListener("change", () => {
  void applyLanConfig({ auditRemote: lanAuditRemoteInput.checked });
});

lanAdvancedToggle.addEventListener("click", () => {
  lanAdvancedExpanded = !lanAdvancedExpanded;
  // Collapsing puts the token back behind its mask.
  if (!lanAdvancedExpanded) {
    lanTokenRevealed = false;
    lanRotateArmed = false;
  }
  renderLanSettings();
});

const applyLanPort = () => {
  if (lanAdvancedLocked(lanStatus)) return;
  const parsed = parseLanPort(lanPortInput.value);
  if (!parsed.ok) {
    setLanNotice(parsed.error);
    lanPortInput.focus({ preventScroll: true });
    return;
  }
  if (parsed.port === lanConfig.port) {
    setLanNotice();
    return;
  }
  void applyLanConfig({ port: parsed.port });
};

lanPortApplyButton.addEventListener("click", applyLanPort);

lanPortInput.addEventListener("keydown", (event) => {
  if (event.key !== "Enter") return;
  event.preventDefault();
  applyLanPort();
});

lanTokenRevealButton.addEventListener("click", () => {
  lanTokenRevealed = !lanTokenRevealed;
  renderLanSettings();
});

lanTokenCopyButton.addEventListener("click", () => {
  if (!lanConfig.token) return;
  void copyLanText(lanConfig.token).then((copied) => {
    setLanNotice(copied ? undefined : "复制失败，请手动选择令牌文本");
    if (copied) flashLanAction(lanTokenCopyButton, "已复制");
  });
});

lanTokenRotateButton.addEventListener("click", () => {
  if (lanBusy || lanAdvancedLocked(lanStatus)) return;
  if (!lanRotateArmed) {
    lanRotateArmed = true;
    setLanNotice(ROTATE_CONFIRM_HINT, "warn");
    renderLanSettings();
    return;
  }
  lanRotateArmed = false;
  lanBusy = true;
  setLanNotice();
  renderLanSettings();
  const rotation = isTauriRuntime
    ? invoke<string>("lan_rotate_token")
    : Promise.resolve(previewLanToken());
  void rotation
    .then((token) => {
      lanConfig = { ...lanConfig, token };
      lanTokenRevealed = false;
    })
    .catch((error: unknown) => {
      setLanNotice("轮换令牌失败：" + String(error));
    })
    .finally(() => {
      lanBusy = false;
      renderLanSettings();
      void refreshLanStatus();
    });
});

renderLanSettings();

const syncTransparencyControls = () => {
  syncTransparencyControl(
    transparencyInput,
    transparencyOutput,
    appearanceSettings.transparency,
  );
  syncTransparencyControl(
    cardTransparencyInput,
    cardTransparencyOutput,
    appearanceSettings.cardTransparency,
  );
  syncTransparencyControl(
    textTransparencyInput,
    textTransparencyOutput,
    appearanceSettings.textTransparency,
  );
};

const setWorkingSquareImageStatus = (message?: string, tone?: "error") => {
  if (!workingSquareImageStatus) return;
  workingSquareImageStatus.textContent = message ?? "";
  workingSquareImageStatus.hidden = !message;
  if (tone) workingSquareImageStatus.dataset.tone = tone;
  else delete workingSquareImageStatus.dataset.tone;
};

const syncWorkingSquareImagePreviews = () => {
  for (const button of workingSquareImageButtons) {
    const theme = workingSquareImageThemeForTarget(
      button.dataset.workingSquareImageTheme,
      systemThemePreference.matches,
    );
    if (!theme) continue;
    button.style.backgroundImage = cssImageUrl(workingSquareImageUrlFor(theme));
    button.dataset.custom = String(
      Boolean(
        workingSquareImageFileFor(theme) &&
        workingSquareImageStore.urlFor(theme),
      ),
    );
  }
};

const loadWorkingSquareImages = async () => {
  const themes: WorkingSquareImageTheme[] = ["light", "dark"];
  await Promise.all(
    themes.map((theme) =>
      workingSquareImageFileFor(theme)
        ? workingSquareImageStore.load(theme)
        : Promise.resolve(undefined),
    ),
  );
  applyAppearanceSettings(false);
  syncWorkingSquareImagePreviews();
};

const syncAppearanceControls = () => {
  for (const input of themeModeInputs) {
    input.checked = input.value === appearanceSettings.theme;
  }
  for (const input of motionModeInputs) {
    input.checked = input.value === appearanceSettings.motionMode;
  }
  const startupAnimationLocked = appearanceSettings.motionMode === "off";
  const displayedStartupAnimationMode = startupAnimationLocked
    ? "none"
    : appearanceSettings.startupAnimationMode;
  startupAnimationModeFieldset.disabled = startupAnimationLocked;
  for (const input of startupAnimationModeInputs) {
    input.checked = input.value === displayedStartupAnimationMode;
  }
  syncWorkingSquareImagePreviews();
  syncAnimationSpeedControl();
  syncTransparencyControls();
};

const syncSettingsPanel = (selectedItem: HTMLButtonElement) => {
  const section = selectedItem.dataset.settingsSection ?? "general";
  const showsGeneralSettings = section === "general";
  const showsHookSettings = section === "hook";
  const showsThemeSettings = section === "theme";
  const showsSoundSettings = section === "sound";
  const showsLanSettings = section === "lan";
  const showsAboutSettings = section === "about";
  settingsPanel.setAttribute("aria-labelledby", selectedItem.id);
  settingsPlaceholder.hidden =
    showsGeneralSettings ||
    showsHookSettings ||
    showsThemeSettings ||
    showsSoundSettings ||
    showsLanSettings ||
    showsAboutSettings;
  generalSettings.hidden = !showsGeneralSettings;
  if (hookSettings) hookSettings.hidden = !showsHookSettings;
  themeSettings.hidden = !showsThemeSettings;
  soundSettings.hidden = !showsSoundSettings;
  lanSettings.hidden = !showsLanSettings;
  aboutSettings.hidden = !showsAboutSettings;
  settingsPlaceholderText.textContent =
    settingsSectionPlaceholders[section] ??
    `${settingsSectionNames[section] ?? "此分类"}设置内容待添加`;
  if (showsHookSettings) void refreshHookIntegrations();
  if (showsAboutSettings && isTauriRuntime) {
    ensureAutoUpdateCheck();
  }
  if (showsLanSettings) {
    void refreshLanStatus();
    startLanStatusPolling();
  } else {
    stopLanStatusPolling();
  }
};

const transitionSettingsPanel = async (selectedItem: HTMLButtonElement) => {
  const targetSection = selectedItem.dataset.settingsSection ?? "general";
  if (targetSection === requestedSettingsSection) return;

  requestedSettingsSection = targetSection;
  const transitionToken = ++settingsPanelTransitionToken;
  settingsPanelTransition?.cancel();
  settingsPanelTransition = undefined;

  if (targetSection === displayedSettingsSection) {
    syncSettingsPanel(selectedItem);
    return;
  }

  const displayedIndex = settingsNavItems.findIndex(
    (item) => item.dataset.settingsSection === displayedSettingsSection,
  );
  const targetIndex = settingsNavItems.indexOf(selectedItem);
  const direction = targetIndex >= displayedIndex ? 1 : -1;

  const outgoingAnimation = settingsPanel.animate(
    [
      { opacity: 1, transform: "translateX(0)" },
      { opacity: 0, transform: `translateX(${-6 * direction}px)` },
    ],
    { duration: 90, easing: "ease-in", fill: "both" },
  );
  settingsPanelTransition = outgoingAnimation;
  await outgoingAnimation.finished.catch(() => undefined);
  if (transitionToken !== settingsPanelTransitionToken) return;

  outgoingAnimation.cancel();
  syncSettingsPanel(selectedItem);
  settingsPanel.scrollTop = 0;
  displayedSettingsSection = targetSection;
  // Switching settings sections can change the panel's measured height. Mark
  // this layout refresh so the native window follows the content smoothly.
  schedulePanelContentRefresh(true);

  const incomingAnimation = settingsPanel.animate(
    [
      { opacity: 0, transform: `translateX(${6 * direction}px)` },
      { opacity: 1, transform: "translateX(0)" },
    ],
    { duration: 140, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
  );
  settingsPanelTransition = incomingAnimation;
  await incomingAnimation.finished.catch(() => undefined);
  if (transitionToken !== settingsPanelTransitionToken) return;
  incomingAnimation.cancel();
  settingsPanelTransition = undefined;
};

const updateAppearanceSettings = (
  nextSettings: Partial<AppearanceSettings>,
) => {
  appearanceSettings = { ...appearanceSettings, ...nextSettings };
  persistAppearanceSettings();
  applyAppearanceSettings();
  syncAppearanceControls();
};

const syncSettingsNavOverflow = () => {
  const hasOverflow = settingsNav.scrollWidth > settingsNav.clientWidth + 1;
  const canScrollLeft = hasOverflow && settingsNav.scrollLeft > 1;
  const canScrollRight =
    hasOverflow &&
    settingsNav.scrollLeft + settingsNav.clientWidth <
      settingsNav.scrollWidth - 1;

  settingsNavShell.dataset.overflowLeft = String(canScrollLeft);
  settingsNavShell.dataset.overflowRight = String(canScrollRight);
};

const syncSettingsNavIndicator = () => {
  const activeItem = activeSettingsNavItem();
  settingsNavIndicator.style.setProperty(
    "--settings-nav-indicator-x",
    `${activeItem.offsetLeft}px`,
  );
  settingsNavIndicator.style.setProperty(
    "--settings-nav-indicator-width",
    `${activeItem.offsetWidth}px`,
  );
};

const revealSettingsNavItem = (item: HTMLButtonElement) => {
  const itemLeft = item.offsetLeft;
  const itemRight = itemLeft + item.offsetWidth;
  const visibleLeft = settingsNav.scrollLeft;
  const visibleRight = visibleLeft + settingsNav.clientWidth;

  if (itemLeft < visibleLeft) {
    settingsNav.scrollTo({
      left: Math.max(0, itemLeft - 8),
      behavior: "smooth",
    });
  } else if (itemRight > visibleRight) {
    settingsNav.scrollTo({
      left: itemRight - settingsNav.clientWidth + 8,
      behavior: "smooth",
    });
  }
};

const selectSettingsNavItem = (
  selectedItem: HTMLButtonElement,
  focus = false,
) => {
  for (const item of settingsNavItems) {
    const selected = item === selectedItem;
    item.setAttribute("aria-selected", String(selected));
    item.tabIndex = selected ? 0 : -1;
  }

  void transitionSettingsPanel(selectedItem);
  syncSettingsNavIndicator();
  revealSettingsNavItem(selectedItem);
  if (focus) selectedItem.focus({ preventScroll: true });
};

settingsNavItems.forEach((item, index) => {
  item.tabIndex = item.getAttribute("aria-selected") === "true" ? 0 : -1;
  item.addEventListener("click", () => selectSettingsNavItem(item));
  item.addEventListener("keydown", (event) => {
    let nextIndex: number | undefined;
    if (event.key === "ArrowRight") {
      nextIndex = (index + 1) % settingsNavItems.length;
    } else if (event.key === "ArrowLeft") {
      nextIndex =
        (index - 1 + settingsNavItems.length) % settingsNavItems.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = settingsNavItems.length - 1;
    }

    if (nextIndex === undefined) return;
    event.preventDefault();
    selectSettingsNavItem(settingsNavItems[nextIndex], true);
  });
});

settingsNav.addEventListener("scroll", syncSettingsNavOverflow, {
  passive: true,
});

const settingsNavDragScroll = new DragScrollController();
let settingsNavDragPointerId: number | undefined;
let settingsNavDragCaptureTarget: HTMLElement | undefined;

const endSettingsNavDrag = (event?: PointerEvent) => {
  if (settingsNavDragPointerId === undefined) return;
  if (event && event.pointerId !== settingsNavDragPointerId) return;

  const pointerId = settingsNavDragPointerId;
  const captureTarget = settingsNavDragCaptureTarget;
  settingsNavDragPointerId = undefined;
  settingsNavDragCaptureTarget = undefined;
  const dragged = settingsNavDragScroll.end();

  if (captureTarget?.hasPointerCapture(pointerId)) {
    captureTarget.releasePointerCapture(pointerId);
  }
  delete settingsNav.dataset.dragging;
  if (dragged) {
    // Swallow the click that closes the gesture so dragging never switches tabs.
    settingsNav.dataset.dragSuppressClick = "true";
  }
};

settingsNav.addEventListener("pointerdown", (event) => {
  if (event.button !== 0 && event.pointerType === "mouse") return;
  if (settingsNavDragPointerId !== undefined) return;

  const maxScrollLeft = settingsNav.scrollWidth - settingsNav.clientWidth;
  const started = settingsNavDragScroll.start({
    pointerX: event.clientX,
    scrollLeft: settingsNav.scrollLeft,
    maxScrollLeft,
  });
  if (!started) return;

  settingsNavDragPointerId = event.pointerId;
  settingsNavDragCaptureTarget =
    event.target instanceof HTMLElement
      ? (event.target.closest<HTMLElement>(".settings-nav__item") ??
        settingsNav)
      : settingsNav;
  delete settingsNav.dataset.dragSuppressClick;
  settingsNavDragCaptureTarget.setPointerCapture(event.pointerId);
});

settingsNav.addEventListener("pointermove", (event) => {
  if (event.pointerId !== settingsNavDragPointerId) return;

  const nextScrollLeft = settingsNavDragScroll.move(event.clientX);
  if (nextScrollLeft === undefined) return;

  event.preventDefault();
  settingsNav.dataset.dragging = "true";
  settingsNav.scrollLeft = nextScrollLeft;
});

settingsNav.addEventListener("pointerup", endSettingsNavDrag);
settingsNav.addEventListener("pointercancel", endSettingsNavDrag);
settingsNav.addEventListener("lostpointercapture", endSettingsNavDrag);

settingsNav.addEventListener(
  "click",
  (event) => {
    if (settingsNav.dataset.dragSuppressClick !== "true") return;
    // Keyboard activation reports detail 0 and must stay clickable.
    if (event.detail === 0) return;
    delete settingsNav.dataset.dragSuppressClick;
    event.preventDefault();
    event.stopPropagation();
  },
  true,
);

settingsNav.addEventListener("dragstart", (event) => {
  event.preventDefault();
});

settingsNavResizeObserver = new ResizeObserver(() => {
  syncSettingsNavIndicator();
  syncSettingsNavOverflow();
});
settingsNavResizeObserver.observe(settingsNav);

const handleLocaleLayoutChange = () => {
  if (localeLayoutFrame !== undefined) {
    window.cancelAnimationFrame(localeLayoutFrame);
  }
  if (nativePositionFrame !== undefined) {
    const animate = nativePositionAnimationPending;
    window.cancelAnimationFrame(nativePositionFrame);
    nativePositionFrame = undefined;
    scheduleNativeWindowPosition(animate);
  }
  if (nativeInterfaceLayoutFrame !== undefined) {
    window.cancelAnimationFrame(nativeInterfaceLayoutFrame);
  }
  localeLayoutFrame = window.requestAnimationFrame(() => {
    localeLayoutFrame = undefined;
    syncSettingsNavIndicator();
    syncSettingsNavOverflow();
    schedulePanelContentRefresh();
    refreshQuestionPreviewClippedState();
    refreshPlanPreviewClippedState();
  });
};

document.addEventListener("codecraft:locale-change", handleLocaleLayoutChange);

soundEnabledInput.addEventListener("change", () => {
  updateSoundPreference({ enabled: soundEnabledInput.checked });
});

soundVolumeInput.addEventListener("input", () => {
  updateSoundPreference({ volume: Number(soundVolumeInput.value) / 100 });
});

for (const input of soundPackInputs) {
  input.addEventListener("change", () => {
    if (!input.checked) return;
    setCustomSoundStatus();
    updateSoundPreference({ pack: input.value as SoundPackId });
  });
}

for (const button of soundPackPreviewButtons) {
  button.addEventListener("click", () => {
    const pack = button.dataset.soundPackPreview as Exclude<
      SoundPackId,
      "custom"
    >;
    const kind: PresetPreviewKind =
      button.dataset.soundPreviewKind === "taskComplete"
        ? "taskComplete"
        : "approval";
    soundPlayer.previewPack(pack, kind);
  });
}

for (const input of customSoundInputs) {
  input.addEventListener("change", () => {
    const event = input.dataset.customSound as SoundEvent;
    const file = input.files?.[0];
    if (!file || !SOUND_EVENTS.includes(event)) return;
    setCustomSoundStatus(`正在保存“${file.name}”…`);
    void soundPlayer
      .saveCustomFile(event, file)
      .then((fileName) => {
        updateSoundPreference({
          pack: "custom",
          customFiles: {
            ...soundPreference.customFiles,
            [event]: fileName,
          },
        });
        setCustomSoundStatus(`已保存“${fileName}”`);
        void soundPlayer.previewCustom(event);
      })
      .catch((error: unknown) => {
        setCustomSoundStatus(`保存自定义音效失败：${String(error)}`);
      })
      .finally(() => {
        input.value = "";
      });
  });
}

for (const button of customSoundPreviewButtons) {
  button.addEventListener("click", () => {
    const event = button.dataset.customSoundPreview as SoundEvent;
    if (!SOUND_EVENTS.includes(event)) return;
    if (!soundPreference.customFiles[event]) {
      setCustomSoundStatus("请先为该事件选择音频文件");
      return;
    }
    setCustomSoundStatus();
    void soundPlayer.previewCustom(event);
  });
}

topDragEnabledInput.addEventListener("change", () => {
  updateWindowPositionSettings(
    {
      topDragEnabled: topDragEnabledInput.checked,
    },
    false,
  );
});

windowPositionAdvancedToggle.addEventListener("click", () => {
  const expanded =
    windowPositionAdvancedToggle.getAttribute("aria-expanded") !== "true";
  windowPositionAdvancedToggle.setAttribute("aria-expanded", String(expanded));
  setSettingsHeightPanelExpanded(windowPositionAdvanced, expanded);
});

windowPositionInput.addEventListener("input", () => {
  const positionPercent = Number(windowPositionInput.value);
  syncNumericRangeControl(
    windowPositionInput,
    windowPositionOutput,
    positionPercent,
    `${positionPercent}%`,
  );
});

windowPositionInput.addEventListener("change", () => {
  updateWindowPositionSettings({
    horizontalPosition: Number(windowPositionInput.value) / 100,
  });
});

for (const button of windowPositionPresetButtons) {
  button.addEventListener("click", () => {
    updateWindowPositionSettings({
      horizontalPosition: Number(button.dataset.positionPreset),
    });
  });
}

for (const input of interfaceSizingModeInputs) {
  input.addEventListener("change", () => {
    if (!input.checked) return;
    updateInterfaceSettings({ mode: input.value as InterfaceSizingMode });
  });
}

interfaceScaleInput.addEventListener("input", () => {
  updateInterfaceSettings({ scale: Number(interfaceScaleInput.value) / 100 });
});

interfaceFontScaleInput.addEventListener("input", () => {
  updateInterfaceSettings({
    customFontScale: Number(interfaceFontScaleInput.value) / 100,
  });
});

interfaceWidthInput.addEventListener("input", () => {
  updateInterfaceSettings({ customWidth: Number(interfaceWidthInput.value) });
});

interfaceMinHeightInput.addEventListener("input", () => {
  const customMinHeight = Number(interfaceMinHeightInput.value);
  updateInterfaceSettings({
    customMinHeight,
    customMaxHeight: Math.max(
      customMinHeight,
      interfaceSettings.customMaxHeight,
    ),
  });
});

interfaceMaxHeightInput.addEventListener("input", () => {
  const customMaxHeight = Number(interfaceMaxHeightInput.value);
  updateInterfaceSettings({
    customMinHeight: Math.min(
      interfaceSettings.customMinHeight,
      customMaxHeight,
    ),
    customMaxHeight,
  });
});

interfacePaddingInput.addEventListener("input", () => {
  updateInterfaceSettings({
    customPadding: Number(interfacePaddingInput.value),
  });
});

let requestedWorkingSquareImageTheme: WorkingSquareImageTheme | undefined;
let workingSquareImageRequestToken = 0;

const resetWorkingSquareImage = async (theme: WorkingSquareImageTheme) => {
  workingSquareImageRequestToken += 1;
  requestedWorkingSquareImageTheme = undefined;
  await workingSquareImageStore.remove(theme);
  updateAppearanceSettings(
    theme === "light"
      ? { workingSquareLightImageFile: null }
      : { workingSquareDarkImageFile: null },
  );
  setWorkingSquareImageStatus("已恢复默认方块。");
};

for (const button of workingSquareImageButtons) {
  button.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    const theme = workingSquareImageThemeForTarget(
      button.dataset.workingSquareImageTheme,
      systemThemePreference.matches,
    );
    if (!theme || !workingSquareImageInput) return;
    workingSquareImageRequestToken += 1;
    requestedWorkingSquareImageTheme = theme;
    setWorkingSquareImageStatus();
    workingSquareImageInput.value = "";
    workingSquareImageInput.click();
  });
}

workingSquareImageInput?.addEventListener("change", () => {
  const file = workingSquareImageInput.files?.[0];
  const theme = requestedWorkingSquareImageTheme;
  const requestToken = workingSquareImageRequestToken;
  requestedWorkingSquareImageTheme = undefined;
  workingSquareImageInput.value = "";
  if (!file || !theme) return;

  const validation = validateWorkingSquareImageFile(file);
  if (validation === "too-large") {
    setWorkingSquareImageStatus(
      "图片文件过大，请选择小于 10 MB 的文件。",
      "error",
    );
    return;
  }
  if (validation === "unsupported") {
    setWorkingSquareImageStatus("请选择 PNG、SVG 或其他图片文件。", "error");
    return;
  }

  void workingSquareImageIsDecodable(file)
    .then(async (decodable) => {
      if (requestToken !== workingSquareImageRequestToken) return;
      if (!decodable) {
        setWorkingSquareImageStatus(
          "无法读取这张图片，请选择其他文件。",
          "error",
        );
        return;
      }
      await workingSquareImageStore.save(theme, file);
      if (requestToken !== workingSquareImageRequestToken) return;
      updateAppearanceSettings(
        theme === "light"
          ? { workingSquareLightImageFile: file.name }
          : { workingSquareDarkImageFile: file.name },
      );
      setWorkingSquareImageStatus("工作方块图片已更新。");
    })
    .catch(() => {
      if (requestToken !== workingSquareImageRequestToken) return;
      setWorkingSquareImageStatus("保存工作方块图片失败，请重试。", "error");
    });
});

for (const input of themeModeInputs) {
  input.addEventListener("change", () => {
    if (!input.checked) return;
    updateAppearanceSettings({ theme: input.value as ThemeMode });
  });
}

for (const input of motionModeInputs) {
  input.addEventListener("change", () => {
    if (!input.checked) return;
    updateAppearanceSettings({ motionMode: input.value as MotionMode });
  });
}

for (const input of startupAnimationModeInputs) {
  input.addEventListener("change", () => {
    if (!input.checked || startupAnimationModeFieldset.disabled) return;
    updateAppearanceSettings({
      startupAnimationMode: input.value as StartupAnimationMode,
    });
  });
}

animationSpeedInput.addEventListener("input", () => {
  updateAppearanceSettings({
    animationSpeed: clampAnimationSpeed(Number(animationSpeedInput.value)),
  });
});

transparencyInput.addEventListener("input", () => {
  updateAppearanceSettings({
    transparency: clampTransparency(Number(transparencyInput.value)),
  });
});

cardTransparencyInput.addEventListener("input", () => {
  updateAppearanceSettings({
    cardTransparency: clampTransparency(
      Number(cardTransparencyInput.value),
      appearanceSettings.cardTransparency,
    ),
  });
});

textTransparencyInput.addEventListener("input", () => {
  updateAppearanceSettings({
    textTransparency: clampTransparency(
      Number(textTransparencyInput.value),
      appearanceSettings.textTransparency,
    ),
  });
});

const handleSystemMotionChange = () => {
  if (appearanceSettings.motionMode === "system") applyAppearanceSettings();
};

const handleSystemThemeChange = () => {
  if (appearanceSettings.theme === "system") applyAppearanceSettings();
  syncWorkingSquareImagePreviews();
};

systemMotionPreference.addEventListener("change", handleSystemMotionChange);
systemThemePreference.addEventListener("change", handleSystemThemeChange);
syncAppearanceControls();
void loadWorkingSquareImages();
syncInterfaceControls();
syncWindowPositionControls();
scheduleNativeWindowPosition(false);
scheduleNativeInterfaceLayout();
void refreshApprovalSettings();
const initialSettingsNavItem = activeSettingsNavItem();
displayedSettingsSection =
  initialSettingsNavItem.dataset.settingsSection ?? "general";
requestedSettingsSection = displayedSettingsSection;
syncSettingsPanel(initialSettingsNavItem);

settingsOpenButton.addEventListener("click", () => {
  switchContentView("settings");
  settingsBackButton.focus({ preventScroll: true });
  window.requestAnimationFrame(() => {
    syncSettingsNavIndicator();
    syncSettingsNavOverflow();
  });
});

settingsBackButton.addEventListener("click", () => {
  switchContentView("sessions");
  window.requestAnimationFrame(() => {
    settingsOpenButton.focus({ preventScroll: true });
  });
});

const renderUnifiedSessionList = (
  list: HTMLUListElement,
  sessions: UnifiedSession[],
  items: Map<string, HTMLLIElement>,
) => {
  if (sessions.length === 0) {
    const currentEmpty = list.firstElementChild;
    if (
      list.childElementCount !== 1 ||
      !(currentEmpty instanceof HTMLLIElement) ||
      !currentEmpty.classList.contains("session-list__empty")
    ) {
      const empty = document.createElement("li");
      empty.className = "session-list__empty";
      empty.textContent = "暂无会话";
      list.replaceChildren(empty);
    }
    items.clear();
    return;
  }
  const nextItems = new Map<string, HTMLLIElement>();
  for (const session of sessions) {
    const key = unifiedSessionKey(session);
    const item = items.get(key) ?? createSessionButton(session);
    updateSessionButton(item, session);
    nextItems.set(key, item);
  }
  for (const stale of items.values()) stale.remove();
  // The empty state is included in the initial HTML so the card has a useful
  // fallback before the first snapshot arrives. Once sessions are rendered,
  // remove every empty-state node so it cannot remain alongside a session
  // item (including after repeated refreshes).
  list
    .querySelectorAll<HTMLElement>(".session-list__empty")
    .forEach((empty) => empty.remove());
  const orderedItems = sessions.map(
    (session) => nextItems.get(unifiedSessionKey(session))!,
  );
  // Keep the existing list children in place whenever possible. Replacing the
  // whole child list on every stream update briefly removes the hovered button
  // from the DOM, which makes :hover flicker between its hover and base state.
  orderedItems.forEach((item, index) => {
    if (list.children[index] !== item) {
      list.insertBefore(item, list.children[index] ?? null);
    }
  });
  items.clear();
  nextItems.forEach((item, id) => items.set(id, item));
};

const createSessionButton = (session: UnifiedSession): HTMLLIElement => {
  const listItem = document.createElement("li");
  listItem.className = "session-list__item";

  const button = document.createElement("button");
  button.className = "session-button";
  button.type = "button";
  button.dataset.sessionId = session.id;
  button.dataset.sessionKey = unifiedSessionKey(session);
  button.dataset.sessionSource = unifiedSessionSource(session);
  button.dataset.sessionStatus = session.status;
  button.setAttribute(
    "aria-pressed",
    String(
      isCodexSession(session)
        ? session.id === selectedCodexSessionId
        : isOpenCodeSession(session)
          ? openCodeSessionKey(session) === selectedOpenCodeSessionKey
          : session.id === selectedSessionId,
    ),
  );
  button.title = `${unifiedSessionStatusLabel(session)} · ${session.title}`;
  button.addEventListener("click", () =>
    isCodexSession(session)
      ? setSelectedCodexSession(session.id)
      : isOpenCodeSession(session)
        ? setSelectedOpenCodeSession(openCodeSessionKey(session))
        : setSelectedSession(session.id),
  );

  const statusIcon = createPixelStatusSvg(session.status, "pixel-status");

  const statusLabel = document.createElement("span");
  statusLabel.className = "sr-only";
  statusLabel.textContent = unifiedSessionStatusLabel(session);

  const title = document.createElement("span");
  title.className = "session-button__title";
  title.textContent = session.title;

  const liveContent = unifiedSessionLiveContent(session);
  const content = document.createElement("span");
  content.className = "session-button__content";
  content.dataset.contentKind = liveContent.kind;
  content.textContent = liveContent.text;
  content.title = liveContent.text;

  const time = document.createElement("time");
  time.className = "session-button__time";
  time.dateTime = new Date(session.startedAt).toISOString();
  time.textContent = formatSessionTime(session.startedAt);

  const chevron = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  chevron.classList.add("session-button__chevron");
  chevron.setAttribute("viewBox", "0 0 24 24");
  chevron.setAttribute("fill", "none");
  chevron.setAttribute("stroke", "currentColor");
  chevron.setAttribute("stroke-width", "2");
  chevron.setAttribute("stroke-linecap", "round");
  chevron.setAttribute("stroke-linejoin", "round");
  chevron.setAttribute("aria-hidden", "true");
  const chevronPath = document.createElementNS(
    "http://www.w3.org/2000/svg",
    "path",
  );
  chevronPath.setAttribute("d", "m9 18 6-6-6-6");
  chevron.append(chevronPath);

  button.append(statusIcon, statusLabel, title, content, time, chevron);
  listItem.append(button);
  return listItem;
};

const updateSessionButton = (
  listItem: HTMLLIElement,
  session: UnifiedSession,
) => {
  const button = listItem.querySelector<HTMLButtonElement>(".session-button");
  const statusLabel = button?.querySelector<HTMLSpanElement>(".sr-only");
  const title = button?.querySelector<HTMLSpanElement>(
    ".session-button__title",
  );
  const content = button?.querySelector<HTMLSpanElement>(
    ".session-button__content",
  );
  const time = button?.querySelector<HTMLTimeElement>(".session-button__time");
  const statusIcon = button?.querySelector<SVGElement>(".pixel-status");
  if (!button || !statusLabel || !title || !content || !time || !statusIcon) {
    return listItem;
  }

  if (button.dataset.sessionStatus !== session.status) {
    button.dataset.sessionStatus = session.status;
  }
  button.dataset.sessionKey = unifiedSessionKey(session);
  button.dataset.sessionSource = unifiedSessionSource(session);
  button.setAttribute(
    "aria-pressed",
    String(
      isCodexSession(session)
        ? session.id === selectedCodexSessionId
        : isOpenCodeSession(session)
          ? openCodeSessionKey(session) === selectedOpenCodeSessionKey
          : session.id === selectedSessionId,
    ),
  );
  button.title = `${unifiedSessionStatusLabel(session)} · ${session.title}`;
  statusLabel.textContent = unifiedSessionStatusLabel(session);
  title.textContent = session.title;
  const liveContent = unifiedSessionLiveContent(session);
  content.dataset.contentKind = liveContent.kind;
  content.textContent = liveContent.text;
  content.title = liveContent.text;
  statusIcon.dataset.status = session.status;
  time.dateTime = new Date(session.startedAt).toISOString();
  time.textContent = formatSessionTime(session.startedAt);

  return listItem;
};

const renderSessionSnapshot = (snapshot: ClaudeSessionSnapshot) => {
  snapshot = {
    ...snapshot,
    sessions: filterAutoCleanedSessions(
      filterDismissedSessions(
        snapshot.sessions,
        dismissedSessionKeys,
        (session) => `claude:${session.id}`,
        unifiedSessionIsRunning,
      ),
      sessionCleanupSettings,
    ),
  };
  observeClaudeSounds(snapshot.sessions);
  latestClaudeSnapshot = snapshot;
  latestSessions = snapshot.sessions;
  integrationState.title =
    snapshot.integrationError ?? "Claude Code Hook 已连接";
  setSourceStatusLabel(
    claudeConnectionStatus,
    snapshot.connected ? "Hook 正常" : "Hook 异常",
  );
  claudeConnectionStatus.dataset.connected = String(snapshot.connected);
  claudeConnectionStatus.disabled = snapshot.connected;
  claudeConnectionStatus.title = snapshot.connected
    ? "Claude Code Hook 连接正常"
    : (snapshot.integrationError ?? "点击重新安装并连接 Claude Code Hook");
  renderSessionSummary();

  if (snapshot.sessions.length === 0) {
    renderUnifiedSessionList(sessionList, [], sessionItems);
    selectedSessionId = undefined;
    renderedDetailSignature = undefined;
  } else {
    if (
      !snapshot.sessions.some((session) => session.id === selectedSessionId)
    ) {
      selectedSessionId = undefined;
      renderedDetailSignature = undefined;
    }
    renderUnifiedSessionList(sessionList, snapshot.sessions, sessionItems);
    const selectedSession = snapshot.sessions.find(
      (session) => session.id === selectedSessionId,
    );
    if (selectedSession) {
      renderSessionDetail(selectedSession);
    }
  }
  syncCollapsedSessionState();

  const permissionResolution = resolvePendingPermissionRequest(
    snapshot.sessions.map((session) => session.permission),
    locallySubmittedPermissionRequestId ?? manuallyHiddenPermissionRequestId,
    locallySubmittedPermissionRequestId,
  );
  if (
    locallySubmittedPermissionRequestId &&
    !permissionResolution.dismissedRequestId
  ) {
    locallySubmittedPermissionRequestId = undefined;
  }
  manuallyHiddenPermissionRequestId = permissionResolution.dismissedRequestId;

  const dismissedQuestionRequestId =
    locallySubmittedQuestionRequestId ?? manuallyHiddenQuestionRequestId;
  const pendingResolution = resolvePendingQuestionRequest(
    snapshot.sessions.map((session) => session.question),
    dismissedQuestionRequestId,
    locallySubmittedQuestionRequestId
      ? localQuestionSubmissions.get(locallySubmittedQuestionRequestId)
          ?.contentSignature
      : undefined,
  );
  if (locallySubmittedQuestionRequestId) {
    if (!pendingResolution.dismissedRequestId) {
      localQuestionSubmissions.delete(locallySubmittedQuestionRequestId);
    }
    locallySubmittedQuestionRequestId = pendingResolution.dismissedRequestId;
  } else {
    manuallyHiddenQuestionRequestId = pendingResolution.dismissedRequestId;
  }

  const planResolution = resolvePendingPlanRequest(
    snapshot.sessions.map((session) => session.plan),
    locallySubmittedPlanRequestId ?? manuallyHiddenPlanRequestId,
    locallySubmittedPlanRequestId,
  );
  if (locallySubmittedPlanRequestId && !planResolution.dismissedRequestId) {
    locallySubmittedPlanRequestId = undefined;
  }
  manuallyHiddenPlanRequestId = planResolution.dismissedRequestId;

  if (pendingResolution.request) {
    activeQuestionSource = "claude";
    activeCodexQuestionThreadId = undefined;
    if (
      manuallyHiddenQuestionRequestId === pendingResolution.request.id &&
      requestedContentView !== "question"
    ) {
      renderQuestionRequest(pendingResolution.request);
      return;
    }
    manuallyHiddenQuestionRequestId = undefined;
    const shouldAutoReveal =
      pendingResolution.request.id !== lastAutoRevealedQuestionRequestId;
    if (shouldAutoReveal) {
      lastAutoRevealedQuestionRequestId = pendingResolution.request.id;
    }
    if (requestedContentView !== "question") {
      rememberReviewOrigin("question");
    }
    renderQuestionRequest(pendingResolution.request);
    switchContentView("question");

    if (shouldAutoReveal && collapseExpandSettings.approvalAutoExpand) {
      void controller.pointerEntered().catch((error: unknown) => {
        console.error("Unable to reveal the Claude Code question", error);
      });
    }
  } else if (activeQuestionSource === "claude") {
    manuallyHiddenQuestionRequestId = undefined;
    activeQuestionSource = "claude";
    lastAutoRevealedQuestionRequestId = undefined;
    const returnView = returnViewForReview(questionOriginView);
    questionOriginView = undefined;
    clearActiveQuestionRequest();
    if (shouldRestoreReviewOrigin("question", requestedContentView)) {
      switchContentView(returnView);
    }
  }

  if (planResolution.request) {
    if (
      manuallyHiddenPlanRequestId === planResolution.request.id &&
      requestedContentView !== "plan"
    ) {
      renderPlanRequest(planResolution.request);
      return;
    }
    manuallyHiddenPlanRequestId = undefined;
    const shouldAutoReveal =
      planResolution.request.id !== lastAutoRevealedPlanRequestId;
    if (shouldAutoReveal) {
      lastAutoRevealedPlanRequestId = planResolution.request.id;
    }
    if (requestedContentView !== "plan") {
      rememberReviewOrigin("plan");
    }
    renderPlanRequest(planResolution.request);
    switchContentView("plan");
    refreshPlanPreviewClippedState();

    if (shouldAutoReveal && collapseExpandSettings.approvalAutoExpand) {
      void controller.pointerEntered().catch((error: unknown) => {
        console.error("Unable to reveal the Claude Code plan request", error);
      });
    }
  } else if (activePlanSource === "claude") {
    manuallyHiddenPlanRequestId = undefined;
    lastAutoRevealedPlanRequestId = undefined;
    const returnView = returnViewForReview(planOriginView);
    planOriginView = undefined;
    clearActivePlanRequest();
    if (shouldRestoreReviewOrigin("plan", requestedContentView)) {
      switchContentView(returnView);
    }
  }

  if (permissionResolution.request && requestedContentView !== "plan") {
    activePermissionSource = "claude";
    activeCodexQuestionThreadId = undefined;
    if (
      manuallyHiddenPermissionRequestId === permissionResolution.request.id &&
      requestedContentView !== "permission"
    ) {
      renderPermissionRequest(permissionResolution.request);
      return;
    }
    manuallyHiddenPermissionRequestId = undefined;
    const shouldAutoReveal =
      permissionResolution.request.id !== lastAutoRevealedPermissionRequestId;
    if (shouldAutoReveal) {
      lastAutoRevealedPermissionRequestId = permissionResolution.request.id;
    }
    if (requestedContentView !== "permission") {
      rememberReviewOrigin("permission");
    }
    renderPermissionRequest(permissionResolution.request);
    switchContentView("permission");

    if (shouldAutoReveal && collapseExpandSettings.approvalAutoExpand) {
      void controller.pointerEntered().catch((error: unknown) => {
        console.error(
          "Unable to reveal the Claude Code permission request",
          error,
        );
      });
    }
  } else if (activePermissionSource === "claude") {
    manuallyHiddenPermissionRequestId = undefined;
    activePermissionSource = "claude";
    lastAutoRevealedPermissionRequestId = undefined;
    const returnView = returnViewForReview(permissionOriginView);
    permissionOriginView = undefined;
    clearActivePermissionRequest();
    if (shouldRestoreReviewOrigin("permission", requestedContentView)) {
      switchContentView(returnView);
    }
  }
};

const refreshClaudeSessions = async () => {
  if (refreshingSessions || !isTauriRuntime) return;
  refreshingSessions = true;

  try {
    const snapshot = await invoke<ClaudeSessionSnapshot>(
      "list_claude_sessions",
    );
    renderSessionSnapshot(snapshot);
    lastRefreshError = undefined;
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    renderSessionSnapshot({
      connected: false,
      integrationError: message,
      sessions: [],
    });
    if (message !== lastRefreshError) {
      console.error("Unable to refresh Claude Code sessions", error);
      lastRefreshError = message;
    }
  } finally {
    refreshingSessions = false;
  }
};

let panelContentResizeFrame: number | undefined;
let panelContentResizeAnimate = false;

const refreshPanelForContent = () => {
  panelContentResizeFrame = undefined;
  const animateHeight = panelContentResizeAnimate;
  panelContentResizeAnimate = false;
  if (!refreshExpandedPanelHeight()) return;

  syncPanelShape();
  void controller.contentResized(animateHeight).catch((error: unknown) => {
    console.error("Unable to resize the CodeCraft panel", error);
  });
};

const schedulePanelContentRefresh = (animateHeight = false) => {
  panelContentResizeAnimate ||= animateHeight;
  if (panelContentResizeFrame !== undefined) return;

  panelContentResizeFrame = window.requestAnimationFrame(
    refreshPanelForContent,
  );
};

const panelContentResizeObserver = new ResizeObserver(() => {
  schedulePanelContentRefresh();
});

panelContentResizeObserver.observe(sessionView);
panelContentResizeObserver.observe(sessionDetailView);
panelContentResizeObserver.observe(settingsView);
panelContentResizeObserver.observe(questionView);
panelContentResizeObserver.observe(permissionView);
panelContentResizeObserver.observe(planView);

const panelContentMutationObserver = new MutationObserver(() => {
  schedulePanelContentRefresh();
});

panelContentMutationObserver.observe(panelBody, {
  attributes: true,
  attributeFilter: ["data-collapsed", "hidden"],
  characterData: true,
  childList: true,
  subtree: true,
});

window.addEventListener("resize", () => {
  refreshQuestionPreviewClippedState();
  refreshPlanPreviewClippedState();
});
if (document.fonts) {
  void document.fonts.ready.then(() => {
    refreshQuestionPreviewClippedState();
    refreshPlanPreviewClippedState();
  });
}

interface CodeCraftContextData {
  kind: "panel" | "session" | "working-square";
  sessionButton?: HTMLButtonElement;
  sessionId?: string;
  sessionKey?: string;
  sessionSource?: "claude" | "codex" | "opencode";
  workingSquareTheme?: WorkingSquareImageTheme;
}

// Closing the in-panel context menu can expose the panel underneath the
// pointer and emit a fresh pointerenter. Keep an explicit collapse stable
// until that pointer has actually left the panel.
let suppressPanelRevealUntilPointerLeave = false;

const copyContextValue = async (value: string) => {
  try {
    await navigator.clipboard.writeText(value);
  } catch (error: unknown) {
    console.error("Unable to copy context menu value", error);
  }
};

const dismissSessionFromList = (data: CodeCraftContextData | undefined) => {
  if (data?.kind !== "session" || !data.sessionId || !data.sessionSource) {
    return;
  }
  const sessionKey = data.sessionKey ?? data.sessionId;
  dismissedSessionKeys.add(`${data.sessionSource}:${sessionKey}`);

  if (
    (data.sessionSource === "claude" && selectedSessionId === data.sessionId) ||
    (data.sessionSource === "codex" && selectedCodexSessionId === data.sessionId) ||
    (data.sessionSource === "opencode" &&
      selectedOpenCodeSessionKey === sessionKey)
  ) {
    selectedSessionId = undefined;
    selectedCodexSessionId = undefined;
    selectedOpenCodeSessionKey = undefined;
    renderedDetailSignature = undefined;
    if (requestedContentView === "detail") switchContentView("sessions");
  }

  if (data.sessionSource === "claude") {
    renderSessionSnapshot({
      ...latestClaudeSnapshot,
      sessions: latestClaudeSnapshot.sessions.filter(
        (session) => session.id !== data.sessionId,
      ),
    });
  } else if (data.sessionSource === "codex") {
    renderCodexSnapshot({
      ...latestCodexSnapshot,
      sessions: latestCodexSnapshot.sessions.filter(
        (session) => session.id !== data.sessionId,
      ),
      interactions: latestCodexSnapshot.interactions.filter(
        (interaction) => interaction.threadId !== data.sessionId,
      ),
    });
  } else {
    renderOpenCodeSnapshot({
      ...latestOpenCodeSnapshot,
      sessions: latestOpenCodeSnapshot.sessions.filter(
        (session) => openCodeSessionKey(session) !== sessionKey,
      ),
    });
  }
};

const contextMenu = new ContextMenuController<CodeCraftContextData>(
  document.body,
  {
    ariaLabel: "CodeCraft 快捷操作",
    onClose: (reason) => {
      if (reason === "selection") {
        if (!panel.matches(":hover")) {
          suppressPanelRevealUntilPointerLeave = false;
        }
        return;
      }
      if (reason !== "outside" && reason !== "escape" && reason !== "tab") {
        return;
      }
      if (panel.matches(":hover")) return;
      if (
        document.activeElement instanceof Node &&
        panel.contains(document.activeElement)
      ) {
        return;
      }
      controller.pointerLeft();
    },
    getData: (target) => {
      const workingSquareOption = target.closest<HTMLElement>(
        ".theme-switch__option",
      );
      const workingSquareTarget =
        workingSquareOption?.querySelector<HTMLElement>(
          "[data-working-square-image-theme]",
        )?.dataset.workingSquareImageTheme;
      const workingSquareTheme = workingSquareImageThemeForTarget(
        workingSquareTarget,
        systemThemePreference.matches,
      );
      if (workingSquareTheme) {
        return {
          kind: "working-square",
          workingSquareTheme,
        };
      }

      const sessionButton =
        target.closest<HTMLButtonElement>(".session-button");
      const sessionSource = sessionButton?.dataset.sessionSource;
      if (
        sessionButton?.dataset.sessionId &&
        (sessionSource === "claude" ||
          sessionSource === "codex" ||
          sessionSource === "opencode")
      ) {
        return {
          kind: "session",
          sessionButton,
          sessionId: sessionButton.dataset.sessionId,
          sessionKey: sessionButton.dataset.sessionKey,
          sessionSource,
        };
      }
      return { kind: "panel" };
    },
    items: (context): ContextMenuItem<CodeCraftContextData>[] => {
      if (rootElement.hasAttribute("data-panel-live")) return [];

      const isSession = context.data?.kind === "session";
      const isWorkingSquare = context.data?.kind === "working-square";
      const workingSquareTheme = context.data?.workingSquareTheme;
      const collapsed = rootElement.dataset.panelState === "collapsed";
      const liveSession = primaryUnifiedLiveSession(latestUnifiedSessions());
      return [
        {
          id: "reset-working-square",
          label: "重置成默认方块",
          icon: "refresh",
          visible: isWorkingSquare,
          disabled:
            !workingSquareTheme ||
            !workingSquareImageFileFor(workingSquareTheme),
          onSelect: () => {
            if (workingSquareTheme) {
              void resetWorkingSquareImage(workingSquareTheme);
            }
          },
        },
        {
          type: "separator",
          id: "working-square-actions-end",
          visible: isWorkingSquare,
        },
        {
          id: "open-session",
          label: "打开会话",
          icon: "arrow-up-right",
          shortcut: "Enter",
          visible: isSession,
          onSelect: () => context.data?.sessionButton?.click(),
        },
        {
          id: "copy-session-id",
          label: "复制会话 ID",
          icon: "copy",
          visible: isSession,
          disabled: !context.data?.sessionId,
          onSelect: () => {
            const sessionId = context.data?.sessionId;
            if (sessionId) void copyContextValue(sessionId);
          },
        },
        {
          type: "separator",
          id: "session-actions-end",
          visible: isSession,
        },
        {
          id: "refresh-sessions",
          label: "刷新会话",
          icon: "refresh",
          shortcut: "R",
          disabled: !isTauriRuntime,
          onSelect: () => {
            void refreshClaudeSessions();
            void refreshCodexPanel();
            void refreshOpenCodeSessions();
          },
        },
        {
          id: "delete-session",
          label: "删除会话",
          icon: "trash",
          danger: true,
          visible: isSession,
          disabled: !context.data?.sessionId,
          onSelect: (menuContext) => dismissSessionFromList(menuContext.data),
        },
        { type: "separator", id: "panel-actions-end" },
        {
          id: "collapse-mini",
          label: "收起成小界面",
          icon: "minus",
          visible: !collapsed && liveSession !== undefined,
          onSelect: () => {
            suppressPanelRevealUntilPointerLeave = true;
            const operation = controller.collapse("mini");
            void operation.catch((error: unknown) => {
              console.error(
                "Unable to collapse the CodeCraft panel to its live view",
                error,
              );
            });
          },
        },
        {
          id: "toggle-panel",
          label: collapsed ? "展开面板" : "收起面板",
          icon: collapsed ? "arrow-up-right" : "minus",
          onSelect: () => {
            if (!collapsed) suppressPanelRevealUntilPointerLeave = true;
            const operation = collapsed
              ? controller.pointerEntered()
              : controller.collapse();
            void operation.catch((error: unknown) => {
              console.error("Unable to toggle the CodeCraft panel", error);
            });
          },
        },
      ];
    },
  },
);

syncProductVisibility();
void refreshHookIntegrations();

// Themed hover tooltips replace the native `title` tooltips. The controller
// follows the cursor and keeps the tooltip open while it stays inside the
// source element, so it also works on the collapsed panel and settings.
const tooltip = new TooltipController();

const startupSequencePromise = runStartupSequence();
let stopReopenRequestedListener: UnlistenFn | undefined;

const handleReopenRequest = async () => {
  if (!(await invoke<boolean>("take_reopen_request"))) return;

  await startupSequencePromise;
  if (!(await controller.revealIfCollapsed())) return;

  await invoke("show_panel_for_attention");
};

if (isTauriRuntime) {
  void listen(REOPEN_REQUESTED_EVENT, () => {
    void handleReopenRequest().catch((error: unknown) => {
      console.error("Unable to reopen the CodeCraft panel", error);
    });
  })
    .then((unlisten) => {
      stopReopenRequestedListener = unlisten;
      return handleReopenRequest();
    })
    .catch((error: unknown) => {
      console.error("Unable to register the CodeCraft reopen handler", error);
    });
  initCodexPanel(renderCodexSnapshot);
  void refreshClaudeSessions();
  void refreshCodexPanel();
  void refreshOpenCodeSessions();
  sessionRefreshTimer = setInterval(() => {
    void refreshClaudeSessions();
    void refreshCodexPanel();
    void refreshOpenCodeSessions();
  }, SESSION_REFRESH_INTERVAL_MS);
} else {
  const previewParameters = new URLSearchParams(window.location.search);
  const previewQuestionEnabled = previewParameters.has("previewQuestion");
  const previewPlanEnabled = previewParameters.has("previewPlan");
  const previewCodexPlanEnabled = previewParameters.has("previewCodexPlan");
  const requestedPreviewQuestionCount = Number(
    previewParameters.get("previewQuestionCount") ?? "4",
  );
  const previewQuestionCount = Number.isFinite(requestedPreviewQuestionCount)
    ? Math.min(4, Math.max(1, Math.trunc(requestedPreviewQuestionCount)))
    : 4;
  const previewEnabled =
    previewParameters.has("previewSessions") ||
    previewQuestionEnabled ||
    previewPlanEnabled ||
    previewCodexPlanEnabled;
  const previewStartedAt = Date.now() - 18 * 60 * 1_000;
  const previewQuestions: ClaudeQuestion[] = [
    {
      header: "交互方式",
      question:
        "回答问题的界面应该如何处理多个选项、自由输入和继续讨论？这是用于验证三行截断与完整问题悬浮层的预览文本。",
      multiSelect: false,
      options: [
        {
          label: "直接选择",
          description: "点击一个选项作为当前回答",
        },
        {
          label: "先查看详情",
          description: "阅读完整说明后再做决定",
        },
      ],
    },
    {
      header: "功能范围",
      question: "这个版本需要同时包含哪些交互能力？",
      multiSelect: true,
      options: [
        { label: "多题导航", description: "支持上一步和下一步" },
        { label: "状态保留", description: "往返问题时保留已选答案" },
        { label: "自由输入", description: "选择其他后输入补充内容" },
        { label: "本地提交", description: "提交后返回会话列表" },
      ],
    },
    {
      header: "收起高度",
      question: "回答选项收起后应该保留多少可见内容？",
      multiSelect: false,
      options: [
        { label: "一项高度", description: "最紧凑的显示方式" },
        { label: "两项高度", description: "兼顾信息量和面板高度" },
        { label: "三项高度", description: "收起后仍显示较多内容" },
      ],
    },
    {
      header: "验证方式",
      question: "完成界面重构后优先采用哪一种验证方式？",
      multiSelect: false,
      options: [
        { label: "自动化测试", description: "验证状态流和数据解析" },
        { label: "单题预览", description: "检查一个问题的紧凑布局" },
        { label: "四题预览", description: "检查完整导航和答案恢复" },
        { label: "键盘操作", description: "检查焦点和收起按钮语义" },
      ],
    },
  ];
  renderSessionSnapshot({
    connected: true,
    integrationError: null,
    sessions: previewEnabled
      ? [
          {
            id: "preview-working",
            status:
              previewQuestionEnabled || previewPlanEnabled
                ? "waiting"
                : "working",
            title: "实现 Claude Code 活跃会话 Hook 与状态同步",
            startedAt: previewStartedAt,
            updatedAt: Date.now(),
            question: previewQuestionEnabled
              ? {
                  id: "preview-question",
                  questions: previewQuestions.slice(0, previewQuestionCount),
                }
              : null,
            permission: null,
            plan: previewPlanEnabled
              ? {
                  id: "preview-plan",
                  toolName: "ExitPlanMode",
                  plan: [
                    "# 实行计划",
                    "",
                    "1. 读取当前 Hook 配置，确认 PermissionRequest 已注册",
                    "2. 在 claude_hook.rs 中新增计划请求的捕获与回传",
                    "3. 在前端新增“计划”界面，展示计划内容",
                    "4. 运行 cargo test、tsc 与 vitest 验证",
                  ].join("\n"),
                  cwd: "C:\\work\\CodeCraft",
                  capturedAt: Date.now(),
                }
              : null,
            activities: [
              {
                id: "preview-read",
                tool: "Read",
                summary: "src-tauri/src/claude_hook.rs",
                status: "completed",
                startedAt: Date.now() - 8_000,
                updatedAt: Date.now() - 7_400,
              },
              {
                id: "preview-edit",
                tool: "Edit",
                summary: "src/main.ts",
                status: "running",
                startedAt: Date.now() - 2_800,
                updatedAt: Date.now() - 2_800,
              },
            ],
            outputs: [
              {
                id: "preview-output-1",
                text: "我正在检查现有会话列表和 Hook 数据结构。",
              },
              {
                id: "preview-output-2",
                text: "接下来会把工具活动和转录输出接入会话详情视图。",
              },
            ],
          },
          {
            id: "preview-attention",
            status: "attention",
            title: "等待确认文件修改权限",
            startedAt: previewStartedAt - 8 * 60 * 1_000,
            updatedAt: Date.now() - 60_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
          {
            id: "preview-tool-failed",
            status: "toolFailed",
            title: "运行测试命令",
            startedAt: previewStartedAt - 14 * 60 * 1_000,
            updatedAt: Date.now() - 70_000,
            question: null,
            permission: null,
            plan: null,
            activities: [
              {
                id: "preview-failed-tool",
                tool: "Bash",
                summary: "npm test",
                status: "failed",
                startedAt: Date.now() - 90_000,
                updatedAt: Date.now() - 70_000,
              },
            ],
            outputs: [],
          },
          {
            id: "preview-stopped",
            status: "stopped",
            title: "整理会话状态测试",
            startedAt: previewStartedAt - 20 * 60 * 1_000,
            updatedAt: Date.now() - 110_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
          {
            id: "preview-waiting",
            status: "waiting",
            title: "重构会话列表组件",
            startedAt: previewStartedAt - 24 * 60 * 1_000,
            updatedAt: Date.now() - 120_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
          {
            id: "preview-idle",
            status: "idle",
            title: "CodeCraft",
            startedAt: previewStartedAt - 41 * 60 * 1_000,
            updatedAt: Date.now() - 180_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
          {
            id: "preview-five",
            status: "waiting",
            title: "检查 Hook 配置兼容性",
            startedAt: previewStartedAt - 56 * 60 * 1_000,
            updatedAt: Date.now() - 240_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
          {
            id: "preview-six",
            status: "idle",
            title: "整理会话状态测试",
            startedAt: previewStartedAt - 72 * 60 * 1_000,
            updatedAt: Date.now() - 300_000,
            question: null,
            permission: null,
            plan: null,
            activities: [],
            outputs: [],
          },
        ]
      : [],
  });
  if (previewCodexPlanEnabled) {
    renderCodexSnapshot({
      connected: true,
      integrationError: null,
      version: 1,
      sessions: [
        {
          id: "preview-codex-plan-session",
          status: "waitingForInput",
          title: "CodeCraft Codex Hook",
          cwd: "C:\\work\\CodeCraft",
          startedAt: previewStartedAt,
          updatedAt: Date.now(),
          activities: [],
          outputs: [],
          pendingInteractionId: "preview-codex-plan",
        },
      ],
      interactions: [
        {
          requestId: "preview-codex-plan",
          kind: "plan",
          answerable: false,
          threadId: "preview-codex-plan-session",
          title: "Codex · 实行计划",
          detail: "计划已生成，请前往原 Codex 界面选择是否实行。",
          plan: [
            "# 实行计划",
            "",
            "1. 从 Stop Hook 中识别结构化计划内容",
            "2. 将计划作为只读交互写入 Codex 会话状态",
            "3. 在共享计划视图中展示 Markdown 正文",
            "4. 通过“前往 Codex”切换到对应终端或桌面任务",
          ].join("\n"),
          questions: [],
          allowSession: false,
          isSecret: false,
          resolved: false,
          capturedAt: Date.now(),
        },
      ],
    });
  }
}

let panelDragPointerId: number | undefined;
let panelDragLastScreenX = 0;
let panelDragPendingDelta = 0;
let panelDragMoveInFlight = false;

const flushPanelDragMovement = async () => {
  if (document.documentElement.dataset.panelState !== "expanded") {
    panelDragPendingDelta = 0;
    return;
  }
  if (panelDragMoveInFlight || Math.abs(panelDragPendingDelta) < 0.01) return;
  const delta = panelDragPendingDelta;
  panelDragPendingDelta = 0;
  panelDragMoveInFlight = true;

  try {
    if (isTauriRuntime) {
      const horizontalPosition = await invoke<number>(
        "move_panel_horizontally",
        { delta },
      );
      updateWindowPositionSettings({ horizontalPosition }, false);
    } else {
      updateWindowPositionSettings(
        {
          horizontalPosition:
            windowPositionSettings.horizontalPosition +
            delta / window.innerWidth,
        },
        false,
      );
    }
  } catch (error: unknown) {
    console.error("Unable to drag the CodeCraft panel", error);
  } finally {
    panelDragMoveInFlight = false;
    if (Math.abs(panelDragPendingDelta) >= 0.01) {
      void flushPanelDragMovement();
    }
  }
};

const endPanelPositionDrag = (event: PointerEvent) => {
  if (event.pointerId !== panelDragPointerId) return;
  if (panelDragRegion.hasPointerCapture(event.pointerId)) {
    panelDragRegion.releasePointerCapture(event.pointerId);
  }
  panelDragPointerId = undefined;
  delete panelDragRegion.dataset.dragging;
  void flushPanelDragMovement();
};

panelDragRegion.addEventListener("pointerdown", (event) => {
  if (
    !windowPositionSettings.topDragEnabled ||
    document.documentElement.dataset.panelState !== "expanded" ||
    event.button !== 0
  ) {
    return;
  }
  event.preventDefault();
  panelDragPointerId = event.pointerId;
  panelDragLastScreenX = event.screenX;
  panelDragPendingDelta = 0;
  panelDragRegion.dataset.dragging = "true";
  panelDragRegion.setPointerCapture(event.pointerId);
});

panelDragRegion.addEventListener("pointermove", (event) => {
  if (event.pointerId !== panelDragPointerId) return;
  if (document.documentElement.dataset.panelState !== "expanded") {
    endPanelPositionDrag(event);
    return;
  }
  event.preventDefault();
  panelDragPendingDelta += event.screenX - panelDragLastScreenX;
  panelDragLastScreenX = event.screenX;
  void flushPanelDragMovement();
});

panelDragRegion.addEventListener("pointerup", endPanelPositionDrag);
panelDragRegion.addEventListener("pointercancel", endPanelPositionDrag);

collapseHandle.addEventListener("click", () => {
  void controller.collapse().catch((error: unknown) => {
    console.error("Unable to collapse the CodeCraft panel", error);
  });
  collapseHandle.blur();
});

sessionLiveView.addEventListener("click", () => {
  selectedSessionId = undefined;
  renderedDetailSignature = undefined;
  switchContentView("sessions");
  void controller.pointerEntered().catch((error: unknown) => {
    console.error("Unable to expand the CodeCraft panel", error);
  });
});

sessionLiveCollapse.addEventListener("click", () => {
  void controller.collapse().catch((error: unknown) => {
    console.error("Unable to collapse the CodeCraft panel", error);
  });
  sessionLiveCollapse.blur();
});

panel.addEventListener("pointerenter", () => {
  if (rootElement.hasAttribute("data-startup-phase")) return;
  if (suppressPanelRevealUntilPointerLeave) return;
  if (document.documentElement.hasAttribute("data-panel-live")) return;
  void controller.pointerEntered().catch((error: unknown) => {
    console.error("Unable to expand the CodeCraft panel", error);
  });
});

panel.addEventListener("pointerleave", () => {
  if (rootElement.hasAttribute("data-startup-phase")) return;
  if (panelDragPointerId !== undefined) return;
  if (!contextMenu.element.hidden) return;
  suppressPanelRevealUntilPointerLeave = false;
  if (document.documentElement.dataset.panelState !== "expanded") return;
  if (panel.contains(document.activeElement)) return;
  controller.pointerLeft();
});

panel.addEventListener("focusout", (event) => {
  if (!contextMenu.element.hidden) return;
  if (document.documentElement.dataset.panelState !== "expanded") return;
  const nextTarget = event.relatedTarget;
  if (nextTarget instanceof Node && panel.contains(nextTarget)) return;
  if (panel.matches(":hover")) return;

  controller.pointerLeft();
});

window.addEventListener("beforeunload", () => {
  stopReopenRequestedListener?.();
  contextMenu.destroy();
  tooltip.destroy();
  activeNativeAnimations = 0;
  if (panelShapeFrame !== undefined) {
    window.cancelAnimationFrame(panelShapeFrame);
  }
  if (sessionRefreshTimer !== undefined) {
    clearInterval(sessionRefreshTimer);
  }
  stopLanStatusPolling();
  if (contentViewTransitionTimer !== undefined) {
    clearTimeout(contentViewTransitionTimer);
  }
  if (incomingViewFrame !== undefined) {
    window.cancelAnimationFrame(incomingViewFrame);
  }
  if (sessionCleanupMenuCloseTimer !== undefined) {
    clearTimeout(sessionCleanupMenuCloseTimer);
  }
  if (sessionCleanupMenuOpenFrame !== undefined) {
    window.cancelAnimationFrame(sessionCleanupMenuOpenFrame);
  }

  settingsPanelTransitionToken += 1;
  settingsPanelTransition?.cancel();
  if (questionOptionsMeasureFrame !== undefined) {
    window.cancelAnimationFrame(questionOptionsMeasureFrame);
  }
  if (panelContentResizeFrame !== undefined) {
    window.cancelAnimationFrame(panelContentResizeFrame);
  }
  if (localeLayoutFrame !== undefined) {
    window.cancelAnimationFrame(localeLayoutFrame);
  }
  for (const animation of questionLayoutAnimations.values()) {
    animation.cancel();
  }
  questionLayoutAnimations.clear();
  for (const state of questionTextSwaps.values()) {
    state.animation?.cancel();
  }
  questionTextSwaps.clear();
  for (const [button, state] of actionButtonVisibilityStates) {
    state.animation?.cancel();
    clearActionButtonTransitionStyles(button);
  }
  actionButtonVisibilityStates.clear();
  actionNavVisibilityState?.animation?.cancel();
  actionNavVisibilityState = undefined;
  clearActionNavTransitionStyles();
  cancelExtraVisibilityAnimation();
  window.removeEventListener("resize", syncPanelShape);
  panelContentResizeObserver.disconnect();
  panelContentMutationObserver.disconnect();
  cardSizeAnimator.disconnect();
  settingsNavResizeObserver?.disconnect();
  document.removeEventListener(
    "codecraft:locale-change",
    handleLocaleLayoutChange,
  );
  systemMotionPreference.removeEventListener(
    "change",
    handleSystemMotionChange,
  );
  document.removeEventListener("animationstart", handleAnimationStart, true);
  document.removeEventListener("transitionrun", handleAnimationStart, true);
  Element.prototype.animate = nativeElementAnimate;
  controller.dispose();
  collapsedWorkingIndicator.dispose();
  clearCollapsedWorkingFlow();
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") {
    controller.pointerLeft();
  }
});
