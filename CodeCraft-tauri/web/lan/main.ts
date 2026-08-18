/**
 * LAN web console entry point.
 *
 * Rendering mirrors the desktop panel and reuses its pure logic modules, so the
 * console adds presentation only: session list, detail, and the three review
 * flows. Codex questions and plans are display-only because the Hook-only
 * integration cannot accept answers from here.
 */

import { formatSessionTime, questionAnswerOptions } from "../../src/claude-sessions";
import { renderMarkdown } from "../../src/plan-markdown";
import {
  createLocalQuestionSubmission,
  createQuestionDrafts,
  isQuestionAnswered,
  updateQuestionSelection,
  type QuestionDraft,
} from "../../src/question-flow";
import * as api from "./api";
import {
  connectionLabel,
  initialConnectionState,
  isAuthenticated,
  nextConnectionState,
  reconnectDelay,
  tokenLooksValid,
  type ConnectionEvent,
  type ConnectionState,
} from "./connection";
import {
  freshnessLabel,
  layoutFor,
  shouldAutoSelectFirst,
  type ConsoleView,
} from "./layout";
import {
  animate,
  clampSpeed,
  enterAnimation,
  loadMotion,
  saveMotion,
  scaledDuration,
  sheetAnimation,
  sheetExitAnimation,
  swapText,
  viewAnimation,
  type MotionSettings,
} from "./motion";
import {
  isSettingsPlacement,
  loadSettingsPlacement,
  saveSettingsPlacement,
  type SettingsPlacement,
} from "./settings-panel";
import {
  clearRememberedToken,
  loadRememberedToken,
  saveRememberedToken,
} from "./remembered-token";
import {
  canAct,
  findEntry,
  firstPendingEntry,
  mergeSnapshot,
  pendingCount,
  pendingLabel,
  type ConsoleEntry,
  type ConsoleSnapshot,
} from "./snapshot";

const THEME_STORAGE_KEY = "codecraft.lan.theme";

const required = <T extends Element>(selector: string): T => {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error("Missing console element: " + selector);
  return element;
};

const root = document.documentElement;
const gate = required<HTMLElement>("#gate");
const tokenInput = required<HTMLInputElement>("#token-input");
const tokenPaste = required<HTMLButtonElement>("#token-paste");
const tokenSubmit = required<HTMLButtonElement>("#token-submit");
const rememberToken = required<HTMLInputElement>("#remember-token");
const tokenError = required<HTMLElement>("#token-error");
const workspace = required<HTMLElement>("#workspace");
const integrations = required<HTMLElement>("#integrations");
const pendingBadge = required<HTMLElement>("#pending-badge");
const sessionSkeleton = required<HTMLElement>("#session-skeleton");
const sessionList = required<HTMLUListElement>("#session-list");
const sessionEmpty = required<HTMLElement>("#session-empty");
const detailPlaceholder = required<HTMLElement>("#detail-placeholder");
const detailPlaceholderTitle = required<HTMLElement>("#detail-placeholder-title");
const detailPlaceholderCopy = required<HTMLElement>("#detail-placeholder-copy");
const detail = required<HTMLElement>("#detail");
const detailBack = required<HTMLButtonElement>("#detail-back");
const detailTitle = required<HTMLElement>("#detail-title");
const detailStatus = required<HTMLElement>("#detail-status");
const detailMeta = required<HTMLElement>("#detail-meta");
const review = required<HTMLElement>("#review");
const reviewKindLabel = required<HTMLElement>("#review-kind-label");
const reviewTool = required<HTMLElement>("#review-tool");
const reviewSummary = required<HTMLElement>("#review-summary");
const reviewQuestions = required<HTMLElement>("#review-questions");
const reviewNotice = required<HTMLElement>("#review-notice");
const reviewActions = required<HTMLElement>("#review-actions");
const activityList = required<HTMLUListElement>("#activity-list");
const activityEmpty = required<HTMLElement>("#activity-empty");
const outputList = required<HTMLUListElement>("#output-list");
const outputEmpty = required<HTMLElement>("#output-empty");
const actionBar = required<HTMLElement>("#action-bar");
const scrim = required<HTMLElement>("#scrim");
const settingsSheet = required<HTMLElement>("#settings-sheet");
const settingsToggle = required<HTMLButtonElement>("#settings-toggle");
const settingsClose = required<HTMLButtonElement>("#settings-close");
const settingsPlacementSegmented = required<HTMLElement>(
  "#settings-placement-segmented",
);
const themeToggle = required<HTMLButtonElement>("#theme-toggle");
const themeSegmented = required<HTMLElement>("#theme-segmented");
const motionSegmented = required<HTMLElement>("#motion-segmented");
const motionSpeed = required<HTMLInputElement>("#motion-speed");
const motionSpeedRow = required<HTMLElement>("#motion-speed-row");
const signOutButton = required<HTMLButtonElement>("#sign-out");
const connectionChip = required<HTMLElement>("#connection");
const connectionLabelElement = required<HTMLElement>("#connection-label");
const connectionInfo = required<HTMLElement>("#connection-info");
const freshness = required<HTMLElement>("#freshness");
const toast = required<HTMLElement>("#toast");

let connection: ConnectionState = initialConnectionState;
let snapshot: ConsoleSnapshot = {
  generatedAt: 0,
  allowApprovals: false,
  entries: [],
  integrations: [],
};
let motion: MotionSettings = loadMotion(window.localStorage);
let settingsPlacement: SettingsPlacement = loadSettingsPlacement(
  window.localStorage,
);
let selectedKey: string | undefined;
let view: ConsoleView = "list";
let stream: EventSource | undefined;
let reconnectTimer: number | undefined;
let toastTimer: number | undefined;
let submitting = false;
let autoRevealedRequestId: string | undefined;
let questionDrafts: QuestionDraft[] = [];
let questionRequestId: string | undefined;
let renderedReviewSignature: string | undefined;
let themeTransitionTimer: number | undefined;

/* ---------- appearance ---------- */

const applyMotion = () => {
  root.dataset.motion = motion.enabled ? "on" : "off";
  root.style.setProperty("--motion-scale", String(motion.speed));
  motionSpeedRow.hidden = !motion.enabled;
  motionSpeed.value = String(motion.speed);
  syncSegmented(motionSegmented, motion.enabled ? "on" : "off", "motionValue");
};

const applyTheme = (theme: "dark" | "light") => {
  root.dataset.theme = theme;
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // The theme still applies to this page load.
  }
  syncSegmented(themeSegmented, theme, "themeValue");
};

type ViewTransitionDocument = Document & {
  startViewTransition?: (update: () => void) => unknown;
};

const transitionTheme = (theme: "dark" | "light") => {
  if (root.dataset.theme === theme) return;
  const documentWithTransition = document as ViewTransitionDocument;
  if (!motion.enabled || !documentWithTransition.startViewTransition) {
    window.clearTimeout(themeTransitionTimer);
    const previousSurface = getComputedStyle(root)
      .getPropertyValue("--surface-page")
      .trim();
    root.style.setProperty("--theme-transition-from", previousSurface);
    root.dataset.themeTransition = "active";
    applyTheme(theme);
    themeTransitionTimer = window.setTimeout(() => {
      delete root.dataset.themeTransition;
      root.style.removeProperty("--theme-transition-from");
    }, scaledDuration(motion, "view") + 24);
    return;
  }
  try {
    documentWithTransition.startViewTransition(() => applyTheme(theme));
  } catch {
    applyTheme(theme);
  }
};

const applySettingsPlacement = (
  placement: SettingsPlacement,
  persist = true,
) => {
  settingsPlacement = placement;
  root.dataset.settingsPlacement = placement;
  if (persist) saveSettingsPlacement(window.localStorage, placement);
  syncSegmented(
    settingsPlacementSegmented,
    placement,
    "settingsPlacementValue",
  );
};

function syncSegmented(container: HTMLElement, value: string, key: string) {
  const buttons = Array.from(
    container.querySelectorAll<HTMLButtonElement>("button"),
  );
  const indicator = container.querySelector<HTMLElement>(".segmented__indicator");
  for (const button of buttons) {
    const pressed = button.dataset[key] === value;
    button.setAttribute("aria-pressed", String(pressed));
    if (pressed && indicator) {
      indicator.style.setProperty("--segmented-x", button.offsetLeft - 3 + "px");
      indicator.style.setProperty("--segmented-width", button.offsetWidth + "px");
    }
  }
}

/* ---------- layout ---------- */

const syncLayout = () => {
  const hasSelection = Boolean(findEntry(snapshot, selectedKey));
  if (
    shouldAutoSelectFirst(
      window.innerWidth,
      hasSelection,
      snapshot.entries.length,
    )
  ) {
    selectedKey = snapshot.entries[0]?.key;
    questionRequestId = undefined;
    renderedReviewSignature = undefined;
    renderSessionList();
    renderDetail();
  }

  const decision = layoutFor(window.innerWidth, view, Boolean(selectedKey));
  root.dataset.layout = decision.breakpoint;
  detail.dataset.sheet = String(decision.asSheet);

  const entry = findEntry(snapshot, selectedKey);
  const showsDetail = Boolean(entry) && (decision.bothPanes || view === "detail");
  detail.hidden = !showsDetail;
  const showsDetailPlaceholder = decision.bothPanes && !showsDetail;
  detailPlaceholder.hidden = !showsDetailPlaceholder;
  if (showsDetailPlaceholder) {
    if (snapshot.generatedAt === 0) {
      detailPlaceholderTitle.textContent = "正在同步会话";
      detailPlaceholderCopy.textContent = "收到首份会话数据后，这里会自动显示第一项的详情。";
    } else if (snapshot.entries.length === 0) {
      detailPlaceholderTitle.textContent = "暂无会话详情";
      detailPlaceholderCopy.textContent = "新会话出现后会自动同步，并在这里显示详细内容。";
    } else {
      detailPlaceholderTitle.textContent = "选择一个会话";
      detailPlaceholderCopy.textContent =
        "从左侧会话列表中选择一项，这里会显示会话详情与审批内容。";
    }
  }
  const listPane = sessionList.closest<HTMLElement>(".pane--list");
  if (listPane) {
    listPane.hidden = !decision.bothPanes && showsDetail && !decision.asSheet;
  }
  detailBack.hidden = decision.bothPanes;
  const showsDetailSheet = decision.asSheet && showsDetail;
  const settingsOpen = !settingsSheet.hidden;
  scrim.hidden = !showsDetailSheet && !settingsOpen;
  scrim.dataset.settingsOpen = String(settingsOpen);
  syncActionBar();
};

const showToast = (message: string, kind: "info" | "error" = "info") => {
  toast.textContent = message;
  toast.dataset.kind = kind;
  toast.hidden = false;
  animate(toast, enterAnimation(motion));
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toast.hidden = true;
  }, 3600);
};

/* ---------- connection ---------- */

/** The approval flag arrives with each snapshot, so the chip text is rebuilt
 * from both the connection phase and the latest snapshot. */
const syncConnectionInfo = () => {
  connectionInfo.textContent =
    "状态：" +
    connectionLabel(connection) +
    (snapshot.allowApprovals ? " · 可远程审批" : " · 只读");
};

const dispatchConnection = (event: ConnectionEvent) => {
  connection = nextConnectionState(connection, event);
  connectionChip.dataset.phase = connection.phase;
  swapText(connectionLabelElement, connectionLabel(connection), motion);
  gate.hidden = isAuthenticated(connection);
  workspace.hidden = !isAuthenticated(connection);
  if (connection.error) {
    tokenError.textContent = connection.error;
    tokenError.hidden = false;
    gate.classList.add("gate--invalid");
    window.setTimeout(() => gate.classList.remove("gate--invalid"), 360);
  } else {
    tokenError.hidden = true;
  }
  syncConnectionInfo();
  if (isAuthenticated(connection)) syncLayout();
};

const closeStream = () => {
  stream?.close();
  stream = undefined;
};

const scheduleReconnect = () => {
  window.clearTimeout(reconnectTimer);
  reconnectTimer = window.setTimeout(openStream, reconnectDelay(connection.attempt));
};

const openStream = () => {
  closeStream();
  const source = new EventSource("/api/events");
  stream = source;

  source.addEventListener("open", () => {
    dispatchConnection({ type: "streamOpen" });
  });

  source.addEventListener("state", (event) => {
    try {
      applySnapshot(JSON.parse((event as MessageEvent<string>).data));
      if (connection.phase !== "live") {
        dispatchConnection({ type: "streamOpen" });
      }
    } catch {
      // A malformed frame is dropped; the next push replaces it.
    }
  });

  source.addEventListener("error", () => {
    closeStream();
    // An expired session shows up as a stream error, so re-check with a plain
    // request before assuming the network is at fault.
    void api
      .fetchState()
      .then((state) => {
        applySnapshot(state);
        dispatchConnection({ type: "streamLost" });
        scheduleReconnect();
      })
      .catch((error: unknown) => {
        if (error instanceof api.ApiError && error.status === 401) {
          dispatchConnection({ type: "sessionMissing" });
          return;
        }
        dispatchConnection({ type: "streamLost" });
        scheduleReconnect();
      });
  });
};

/* ---------- rendering ---------- */

const renderIntegrations = () => {
  integrations.replaceChildren(
    ...snapshot.integrations.map((integration) => {
      const card = document.createElement("div");
      card.className = "integration";
      card.dataset.connected = String(integration.connected);

      const name = document.createElement("span");
      name.className = "integration__name";
      name.textContent = integration.name;

      const state = document.createElement("span");
      state.className = "integration__state";
      state.textContent = integration.connected
        ? integration.sessionCount + " 个会话"
        : integration.error ?? "未连接";

      card.append(name, state);
      return card;
    }),
  );
};

interface SessionCardView {
  item: HTMLLIElement;
  card: HTMLButtonElement;
  source: HTMLElement;
  title: HTMLElement;
  time: HTMLElement;
  status: HTMLElement;
  pending: HTMLElement;
}

const sessionCardViews = new Map<string, SessionCardView>();

const createSessionCardView = (key: string): SessionCardView => {
  const item = document.createElement("li");
  const card = document.createElement("button");
  card.type = "button";
  card.className = "session-card";
  card.dataset.key = key;

  const row = document.createElement("span");
  row.className = "session-card__row";

  const source = document.createElement("span");
  source.className = "session-card__source";

  const title = document.createElement("span");
  title.className = "session-card__title";

  const time = document.createElement("span");
  time.className = "session-card__time";

  const status = document.createElement("span");
  status.className = "session-card__status";

  const pending = document.createElement("span");
  pending.className = "session-card__pending";
  pending.hidden = true;

  row.append(source, title, time);
  card.append(row, status, pending);
  card.addEventListener("click", () => selectEntry(key, "detail"));
  item.append(card);

  return { item, card, source, title, time, status, pending };
};

const updateSessionCardView = (view: SessionCardView, entry: ConsoleEntry) => {
  view.card.dataset.pending = String(entry.pending !== null);
  view.card.setAttribute("aria-current", String(entry.key === selectedKey));
  swapText(view.source, entry.source === "claude" ? "Claude" : "Codex", motion);
  swapText(view.title, entry.title, motion);
  swapText(view.time, formatSessionTime(entry.updatedAt), motion);
  view.status.dataset.status = entry.status;
  swapText(view.status, entry.statusLabel, motion);
  view.pending.hidden = !entry.pending;
  swapText(view.pending, entry.pending ? pendingLabel(entry.pending) : "", motion);
};

const renderSessionList = () => {
  const count = pendingCount(snapshot);
  swapText(pendingBadge, String(count), motion);
  pendingBadge.dataset.empty = String(count === 0);
  sessionSkeleton.hidden = snapshot.generatedAt > 0;
  sessionEmpty.hidden = snapshot.entries.length > 0 || snapshot.generatedAt === 0;

  const activeKeys = new Set<string>();
  snapshot.entries.forEach((entry, index) => {
    activeKeys.add(entry.key);
    let view = sessionCardViews.get(entry.key);
    if (!view) {
      view = createSessionCardView(entry.key);
      sessionCardViews.set(entry.key, view);
    }
    updateSessionCardView(view, entry);

    const currentItem = sessionList.children.item(index);
    if (currentItem !== view.item) {
      sessionList.insertBefore(view.item, currentItem);
    }
  });

  for (const [key, view] of sessionCardViews) {
    if (activeKeys.has(key)) continue;
    view.item.remove();
    sessionCardViews.delete(key);
  }
};

const renderActivities = (entry: ConsoleEntry) => {
  activityEmpty.hidden = entry.activities.length > 0;
  activityList.replaceChildren(
    ...entry.activities.slice(-24).map((activity) => {
      const item = document.createElement("li");
      item.className = "activity";
      item.dataset.status = activity.status;

      const tool = document.createElement("span");
      tool.className = "activity__tool";
      tool.textContent = activity.tool;

      const summary = document.createElement("span");
      summary.className = "activity__summary";
      summary.textContent = activity.summary;

      item.append(tool, summary);
      return item;
    }),
  );
};

const renderOutputs = (entry: ConsoleEntry) => {
  outputEmpty.hidden = entry.outputs.length > 0;
  outputList.replaceChildren(
    ...entry.outputs.slice(-12).map((output) => {
      const item = document.createElement("li");
      item.className = "output markdown";
      item.innerHTML = renderMarkdown(output.text);
      return item;
    }),
  );
};

const actionButton = (
  label: string,
  variant: "primary" | "default" | "danger",
  onClick: () => void,
) => {
  const button = document.createElement("button");
  button.type = "button";
  button.className =
    "button" +
    (variant === "primary"
      ? " button--primary"
      : variant === "danger"
        ? " button--danger"
        : "");
  button.textContent = label;
  button.disabled = submitting;
  button.addEventListener("click", onClick);
  return button;
};

const guardedSubmit = async (action: () => Promise<unknown>, success: string) => {
  if (submitting) return;
  submitting = true;
  renderReview(findEntry(snapshot, selectedKey));
  try {
    await action();
    showToast(success);
  } catch (error) {
    if (error instanceof api.ApiError && error.status === 401) {
      dispatchConnection({ type: "sessionMissing" });
      closeStream();
      return;
    }
    showToast(error instanceof Error ? error.message : "提交失败", "error");
  } finally {
    submitting = false;
    renderReview(findEntry(snapshot, selectedKey));
  }
};

const renderQuestionOptions = (entry: ConsoleEntry) => {
  const request = entry.pending?.question;
  if (!request) {
    reviewQuestions.replaceChildren();
    return;
  }

  if (questionRequestId !== request.id) {
    questionRequestId = request.id;
    questionDrafts = createQuestionDrafts(request);
  }

  const readOnly = entry.pending?.readOnly === true || !snapshot.allowApprovals;
  const blocks = request.questions.map((question, questionIndex) => {
    const draft = questionDrafts[questionIndex];
    const block = document.createElement("div");
    block.className = "question";

    const prompt = document.createElement("p");
    prompt.className = "question__prompt";
    prompt.textContent = question.question;

    const options = document.createElement("div");
    options.className = "question__options";

    const answerOptions = questionAnswerOptions(request.id, question, questionIndex);
    let needsExtraText = false;
    for (const option of answerOptions) {
      const selected = draft.selectedAnswerIds.has(option.id);
      if (selected && option.kind === "other") needsExtraText = true;

      const button = document.createElement("button");
      button.type = "button";
      button.className = "option";
      button.setAttribute("aria-pressed", String(selected));
      button.disabled = readOnly || submitting;

      const label = document.createElement("span");
      label.className = "option__label";
      label.textContent = option.label;
      button.append(label);

      if (option.description) {
        const description = document.createElement("span");
        description.className = "option__description";
        description.textContent = option.description;
        button.append(description);
      }

      button.addEventListener("click", () => {
        questionDrafts[questionIndex] = updateQuestionSelection(
          question,
          questionDrafts[questionIndex],
          option,
        );
        renderReview(entry);
      });
      options.append(button);
    }

    block.append(prompt, options);

    if (needsExtraText) {
      const extra = document.createElement("div");
      extra.className = "question__extra";
      const textarea = document.createElement("textarea");
      textarea.value = draft.extraText;
      textarea.placeholder = "输入你的回答";
      textarea.disabled = readOnly || submitting;
      textarea.addEventListener("input", () => {
        questionDrafts[questionIndex] = {
          ...questionDrafts[questionIndex],
          extraText: textarea.value,
        };
      });
      extra.append(textarea);
      block.append(extra);
    }

    return block;
  });

  reviewQuestions.replaceChildren(...blocks);
};

const renderReview = (entry: ConsoleEntry | undefined) => {
  const pending = entry?.pending ?? null;
  review.hidden = pending === null;
  reviewActions.replaceChildren();
  if (!entry || !pending) {
    reviewQuestions.replaceChildren();
    questionRequestId = undefined;
    renderedReviewSignature = undefined;
    return;
  }

  const signature = pending.requestId + ":" + String(submitting);
  const changed = signature !== renderedReviewSignature;
  renderedReviewSignature = signature;

  reviewKindLabel.textContent = pendingLabel(pending);
  const actionable = canAct(pending, snapshot.allowApprovals);

  reviewNotice.hidden = actionable;
  if (!actionable) {
    reviewNotice.textContent = pending.readOnly
      ? "此请求来自外部 Codex 会话，只能在原终端或 Codex 界面完成，网页仅供查看。"
      : "桌面端未开启远程审批，网页当前为只读。";
  }

  if (pending.kind === "permission" && pending.permission) {
    reviewTool.textContent = pending.permission.toolName;
    reviewSummary.textContent = pending.permission.summary;
    reviewQuestions.replaceChildren();
    questionRequestId = undefined;
    if (actionable) {
      const isCodex = entry.source === "codex";
      reviewActions.append(
        actionButton("允许一次", "primary", () => {
          void guardedSubmit(
            () =>
              isCodex
                ? api.submitCodexApproval(pending.requestId, "accept")
                : api.submitPermission(pending.requestId, "allow"),
            "已允许一次",
          );
        }),
      );
      if (pending.permission.canAlwaysAllow) {
        reviewActions.append(
          actionButton("始终允许", "default", () => {
            void guardedSubmit(
              () =>
                isCodex
                  ? api.submitCodexApproval(pending.requestId, "acceptForSession")
                  : api.submitPermission(pending.requestId, "allowAlways"),
              "已在本会话中始终允许",
            );
          }),
        );
      }
      reviewActions.append(
        actionButton("拒绝", "danger", () => {
          void guardedSubmit(
            () =>
              isCodex
                ? api.submitCodexApproval(pending.requestId, "decline")
                : api.submitPermission(pending.requestId, "deny"),
            "已拒绝",
          );
        }),
      );
    }
  } else if (pending.kind === "plan" && pending.plan) {
    reviewTool.textContent = pending.plan.toolName;
    reviewSummary.innerHTML = renderMarkdown(pending.plan.plan);
    reviewQuestions.replaceChildren();
    questionRequestId = undefined;
    if (actionable) {
      reviewActions.append(
        actionButton("按计划执行", "primary", () => {
          void guardedSubmit(
            () => api.submitPlan(pending.requestId, null),
            "已开始按计划执行",
          );
        }),
      );
    }
  } else if (pending.kind === "question" && pending.question) {
    reviewTool.textContent = pending.question.questions[0]?.header ?? "需要回答";
    reviewSummary.textContent = "";
    renderQuestionOptions(entry);
    if (actionable) {
      const answered = questionDrafts.every(isQuestionAnswered);
      const submit = actionButton("提交回答", "primary", () => {
        const request = pending.question;
        if (!request) return;
        const submission = createLocalQuestionSubmission(request, questionDrafts);
        void guardedSubmit(
          () => api.submitQuestion(submission.requestId, submission.answers),
          "已提交回答",
        );
      });
      submit.disabled = submit.disabled || !answered;
      reviewActions.append(submit);
    }
  }

  reviewActions.hidden = reviewActions.childElementCount === 0;
  if (changed) animate(review, enterAnimation(motion));
};

const syncActionBar = () => {
  const isPhone = root.dataset.layout === "phone";
  const actions = Array.from(
    reviewActions.querySelectorAll<HTMLButtonElement>("button"),
  );
  const shows = isPhone && !detail.hidden && actions.length > 0;
  actionBar.hidden = !shows;
  if (!shows) {
    actionBar.replaceChildren();
    return;
  }
  // Mirrors the review buttons within thumb reach instead of duplicating logic.
  actionBar.replaceChildren(
    ...actions.map((action) => {
      const clone = action.cloneNode(true) as HTMLButtonElement;
      clone.addEventListener("click", () => action.click());
      return clone;
    }),
  );
};

const renderDetail = () => {
  const entry = findEntry(snapshot, selectedKey);
  if (!entry) {
    detail.hidden = true;
    return;
  }
  swapText(detailTitle, entry.title, motion);
  detailStatus.dataset.status = entry.status;
  swapText(detailStatus, entry.statusLabel, motion);
  detailMeta.textContent = [
    entry.source === "claude" ? "Claude Code" : "Codex",
    entry.cwd ? "目录：" + entry.cwd : undefined,
    "更新于 " + formatSessionTime(entry.updatedAt),
  ]
    .filter(Boolean)
    .join(" · ");

  renderReview(entry);
  renderActivities(entry);
  renderOutputs(entry);
};

const selectEntry = (key: string, nextView: ConsoleView) => {
  const changed = key !== selectedKey;
  selectedKey = key;
  const previousView = view;
  view = nextView;
  if (changed) {
    questionRequestId = undefined;
    renderedReviewSignature = undefined;
  }
  renderSessionList();
  renderDetail();
  syncLayout();
  if (previousView !== view || changed) {
    const step =
      root.dataset.layout === "phone" && view === "detail"
        ? sheetAnimation(motion)
        : viewAnimation(motion, view === "detail" ? "forward" : "backward");
    animate(detail, step);
  }
};

const applySnapshot = (raw: unknown) => {
  snapshot = mergeSnapshot((raw ?? {}) as Record<string, never>);
  if (selectedKey && !findEntry(snapshot, selectedKey)) {
    selectedKey = undefined;
    view = "list";
  }

  // Surface a new request without stealing the screen from an active review.
  const pendingEntry = firstPendingEntry(snapshot);
  if (
    pendingEntry?.pending &&
    pendingEntry.pending.requestId !== autoRevealedRequestId &&
    (!selectedKey || view === "list")
  ) {
    autoRevealedRequestId = pendingEntry.pending.requestId;
    selectEntry(pendingEntry.key, "detail");
    renderIntegrations();
    syncConnectionInfo();
    updateFreshness();
    return;
  }
  if (!pendingEntry) autoRevealedRequestId = undefined;

  renderIntegrations();
  renderSessionList();
  renderDetail();
  syncLayout();
  syncConnectionInfo();
  updateFreshness();
};

const updateFreshness = () => {
  swapText(freshness, freshnessLabel(snapshot.generatedAt, Date.now()), motion);
};

/* ---------- events ---------- */

const connectWithToken = async () => {
  const token = tokenInput.value.trim();
  if (!tokenLooksValid(token)) {
    dispatchConnection({ type: "tokenRejected", message: "令牌格式不正确" });
    return;
  }
  tokenSubmit.disabled = true;
  try {
    await api.authenticate(token);
    if (rememberToken.checked) saveRememberedToken(window.localStorage, token);
    else clearRememberedToken(window.localStorage);
    tokenInput.value = "";
    dispatchConnection({ type: "tokenAccepted" });
    applySnapshot(await api.fetchState());
    openStream();
  } catch (error) {
    dispatchConnection({
      type: "tokenRejected",
      message: error instanceof Error ? error.message : "连接失败",
    });
  } finally {
    tokenSubmit.disabled = false;
  }
};

tokenSubmit.addEventListener("click", () => void connectWithToken());
tokenInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") void connectWithToken();
});

tokenPaste.addEventListener("click", () => {
  void navigator.clipboard
    ?.readText()
    .then((text) => {
      tokenInput.value = text.trim();
      tokenInput.focus();
    })
    .catch(() => showToast("无法读取剪贴板，请手动粘贴", "error"));
});

detailBack.addEventListener("click", () => {
  view = "list";
  syncLayout();
  renderSessionList();
});

let settingsReturnFocus: HTMLElement | undefined;
let settingsClosing = false;

const closeSettings = async (restoreFocus = true) => {
  if (settingsSheet.hidden || settingsClosing) return;
  settingsClosing = true;
  settingsToggle.setAttribute("aria-expanded", "false");
  settingsSheet.getAnimations().forEach((animation) => animation.cancel());
  const exitAnimation = animate(
    settingsSheet,
    sheetExitAnimation(motion, settingsPlacement),
  );
  try {
    await exitAnimation?.finished;
  } catch {
    // A superseding animation may cancel this one; the close still completes.
  }
  settingsSheet.hidden = true;
  settingsClosing = false;
  syncLayout();
  if (restoreFocus) {
    (settingsReturnFocus ?? settingsToggle).focus();
  }
  settingsReturnFocus = undefined;
};

const openSettings = () => {
  if (!settingsSheet.hidden || settingsClosing) return;
  settingsReturnFocus = document.activeElement as HTMLElement | null ?? undefined;
  settingsSheet.hidden = false;
  settingsToggle.setAttribute("aria-expanded", "true");
  applySettingsPlacement(settingsPlacement, false);
  syncLayout();
  animate(settingsSheet, sheetAnimation(motion, settingsPlacement));
  syncSegmented(themeSegmented, root.dataset.theme ?? "dark", "themeValue");
  syncSegmented(motionSegmented, motion.enabled ? "on" : "off", "motionValue");
  syncSegmented(
    settingsPlacementSegmented,
    settingsPlacement,
    "settingsPlacementValue",
  );
  settingsClose.focus();
};

scrim.addEventListener("click", () => {
  if (!settingsSheet.hidden) {
    void closeSettings();
    return;
  }
  view = "list";
  syncLayout();
});

settingsToggle.addEventListener("click", () => {
  if (settingsSheet.hidden) openSettings();
  else void closeSettings();
});

settingsClose.addEventListener("click", () => void closeSettings());

settingsPlacementSegmented.addEventListener("click", (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>(
    "button",
  );
  const value = target?.dataset.settingsPlacementValue;
  if (!isSettingsPlacement(value)) return;
  applySettingsPlacement(value);
  settingsSheet.getAnimations().forEach((animation) => animation.cancel());
  animate(settingsSheet, sheetAnimation(motion, value));
});

themeToggle.addEventListener("click", () => {
  transitionTheme(root.dataset.theme === "light" ? "dark" : "light");
});

themeSegmented.addEventListener("click", (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
  const value = target?.dataset.themeValue;
  if (value === "dark" || value === "light") transitionTheme(value);
});

motionSegmented.addEventListener("click", (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
  const value = target?.dataset.motionValue;
  if (value !== "on" && value !== "off") return;
  motion = { ...motion, enabled: value === "on" };
  saveMotion(window.localStorage, motion);
  applyMotion();
});

motionSpeed.addEventListener("input", () => {
  motion = { ...motion, speed: clampSpeed(Number(motionSpeed.value)) };
  saveMotion(window.localStorage, motion);
  applyMotion();
});

signOutButton.addEventListener("click", () => {
  void closeSettings(false);
  closeStream();
  window.clearTimeout(reconnectTimer);
  void api.signOut().catch(() => undefined);
  clearRememberedToken(window.localStorage);
  rememberToken.checked = false;
  tokenInput.value = "";
  dispatchConnection({ type: "signedOut" });
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (!settingsSheet.hidden) {
    void closeSettings();
    return;
  }
  if (root.dataset.layout === "phone" && !detail.hidden) {
    view = "list";
    syncLayout();
    renderSessionList();
  }
});

window.addEventListener("resize", syncLayout);
window.setInterval(updateFreshness, 1000);

// Reconnect promptly when a phone screen wakes up or the tab regains focus.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;
  if (!isAuthenticated(connection) || stream) return;
  window.clearTimeout(reconnectTimer);
  openStream();
});

/* ---------- bootstrap ---------- */

const storedTheme = (() => {
  try {
    const value = window.localStorage.getItem(THEME_STORAGE_KEY);
    return value === "light" ? "light" : "dark";
  } catch {
    return "dark" as const;
  }
})();

applyTheme(storedTheme);
applyMotion();
applySettingsPlacement(settingsPlacement, false);

const rememberedToken = loadRememberedToken(window.localStorage);
if (tokenLooksValid(rememberedToken)) {
  tokenInput.value = rememberedToken;
  rememberToken.checked = true;
} else if (rememberedToken) {
  clearRememberedToken(window.localStorage);
}

// A live cookie skips the token page entirely.
void api
  .fetchState()
  .then((state) => {
    dispatchConnection({ type: "tokenAccepted" });
    applySnapshot(state);
    openStream();
  })
  .catch(() => {
    dispatchConnection({ type: "sessionMissing" });
  });

const revealShell = () => {
  animate(workspace, enterAnimation(motion));
  window.setTimeout(() => {
    syncSegmented(themeSegmented, root.dataset.theme ?? "dark", "themeValue");
    syncSegmented(motionSegmented, motion.enabled ? "on" : "off", "motionValue");
  }, scaledDuration(motion, "enter"));
};

revealShell();
