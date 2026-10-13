import {
  formatSessionTime,
  type ClaudeActivity,
  type ClaudeOutputEntry,
} from "./claude-sessions";
import { renderMarkdown } from "./plan-markdown";

interface SessionDetailElements {
  activitySummary: HTMLElement;
  activityList: HTMLOListElement;
  output: HTMLElement;
}

interface SessionDetailContent {
  key: string;
  assistantLabel: string;
  activities: readonly (Omit<ClaudeActivity, "status" | "startedAt"> & { status: string })[];
  outputs: readonly ClaudeOutputEntry[];
  appendOutputs?: (host: HTMLElement) => void;
}

const activityStatusLabel = (status: string) => ({
  running: "执行中",
  failed: "失败",
  unknown: "结果未知",
  completed: "已完成",
} as Record<string, string>)[status] ?? "已完成";

/** All desktop agents use the existing detail sections and their shared CSS.
 * Provider adapters supply content, never a second layout or provider skin. */
export function renderSessionDetailContent(
  elements: SessionDetailElements,
  content: SessionDetailContent,
) {
  const { activitySummary, activityList, output } = elements;
  const sameSession = activityList.dataset.sessionKey === content.key;
  const activityScroll = sameSession ? activityList.scrollTop : 0;
  const outputScroll = sameSession ? output.scrollTop : 0;
  const followOutput = !sameSession ||
    output.scrollHeight - output.scrollTop - output.clientHeight < 24;
  const expanded = new Set(sameSession
    ? [...activityList.querySelectorAll<HTMLDetailsElement>("details[open]")]
      .map(detail => detail.dataset.activityId)
    : []);
  const focusedDetail = sameSession && activityList.contains(document.activeElement)
    ? document.activeElement?.closest<HTMLDetailsElement>("details")?.dataset.activityId
    : undefined;
  activityList.dataset.sessionKey = content.key;

  const runningCount = content.activities.filter(activity => activity.status === "running").length;
  activitySummary.textContent = runningCount > 0
    ? `${runningCount} 项执行中`
    : `${content.activities.length} 项记录`;

  if (content.activities.length === 0) {
    const empty = document.createElement("li");
    empty.className = "detail-empty";
    empty.textContent = "等待会话调用工具";
    activityList.replaceChildren(empty);
  } else {
    activityList.replaceChildren(...[...content.activities].reverse().map(activity => {
      const item = document.createElement("li");
      item.className = "activity-item";
      item.dataset.activityStatus = activity.status;

      const status = document.createElement("span");
      status.className = "activity-item__status";
      status.ariaHidden = "true";
      const statusText = document.createElement("span");
      statusText.className = "sr-only";
      statusText.textContent = activityStatusLabel(activity.status);

      const body = document.createElement("div");
      body.className = "activity-item__content";
      const tool = document.createElement("span");
      tool.className = "activity-item__tool";
      tool.textContent = activity.tool;
      const summary = document.createElement("span");
      summary.className = "activity-item__summary";
      summary.textContent = activity.summary;
      summary.title = activity.summary;
      body.append(tool, summary);

      if (activity.details) {
        const details = document.createElement("details");
        details.className = "session-activity-details";
        details.dataset.activityId = activity.id;
        details.open = expanded.has(activity.id);
        const label = document.createElement("summary");
        label.textContent = "查看参数与结果";
        const parameters = document.createElement("pre");
        parameters.textContent = activity.details;
        details.append(label, parameters);
        body.append(details);
      }

      const time = document.createElement("time");
      time.className = "activity-item__time";
      time.dateTime = new Date(activity.updatedAt).toISOString();
      time.textContent = formatSessionTime(activity.updatedAt);
      item.append(status, statusText, body, time);
      return item;
    }));
  }

  if (content.outputs.length === 0) {
    const empty = document.createElement("p");
    empty.className = "detail-empty";
    empty.textContent = "等待会话产生可读取的输出";
    output.replaceChildren(empty);
  } else {
    output.replaceChildren(...content.outputs.map(entry => {
      const article = document.createElement("article");
      article.className = "output-entry markdown-content";
      article.dataset.outputId = entry.id;
      article.innerHTML = renderMarkdown(entry.text);
      if (entry.role) {
        const meta = document.createElement("header");
        meta.className = "detail-section__meta";
        meta.textContent = [
          entry.role === "user" ? "用户" : content.assistantLabel,
          entry.capturedAt === undefined ? "" : formatSessionTime(entry.capturedAt),
        ].filter(Boolean).join(" · ");
        article.prepend(meta);
      }
      return article;
    }));
  }
  content.appendOutputs?.(output);

  // Polling must not pull the reader away from expanded tool arguments or
  // earlier messages. Entering another session starts with its latest output.
  if (focusedDetail) {
    [...activityList.querySelectorAll<HTMLDetailsElement>("details")]
      .find(detail => detail.dataset.activityId === focusedDetail)
      ?.querySelector("summary")?.focus({ preventScroll: true });
  }
  activityList.scrollTop = activityScroll;
  output.scrollTop = followOutput ? output.scrollHeight : outputScroll;
}
