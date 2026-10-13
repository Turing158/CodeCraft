import { currentLocale, translateText } from "./i18n";
import { renderMarkdown } from "./plan-markdown";
import { validateCursorAnswers, type CursorAnswer, type CursorDecision, type CursorRequest } from "./cursor-interactions";
import { cursorToolDisplay, createCursorToolDetails } from "./cursor-tool-display";

const t = (text: string) => translateText(text, currentLocale());
const element = <K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text?: string) => {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};

function action(label: string, className: string, run: () => void, writes = true) {
  const node = element("button", className, t(label));
  node.type = "button";
  if (writes) node.dataset.cursorWrite = "true";
  node.addEventListener("click", run);
  return node;
}

function option(label: string, description: string, className: string, run: () => void) {
  const node = action("", `question-option ${className}`, run);
  node.append(element("span", "question-option__label", label));
  if (description) node.append(element("span", "question-option__description", description));
  node.title = [label, description].filter(Boolean).join(" · ");
  return node;
}

function icon(path: string, viewBox: string) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", viewBox);
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.8");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  const shape = document.createElementNS(svg.namespaceURI, "path");
  shape.setAttribute("d", path);
  svg.append(shape);
  return svg;
}

function header(title: string, badge: string, back?: () => void) {
  const node = element("header", "question-block__header");
  if (back) {
    const nav = element("nav", "question-actions question-card-actions");
    nav.setAttribute("aria-label", t("返回"));
    const button = action("", "question-card-action", back, false);
    button.setAttribute("aria-label", t("返回"));
    button.title = t("返回");
    button.append(icon("m15 18-6-6 6-6", "0 0 24 24"));
    nav.append(button);
    node.append(nav);
  }
  const labels = element("div", "question-block__labels");
  const tag = element("span", "question-header-badge", badge);
  tag.title = badge;
  labels.append(element("h1", "", title), tag);
  node.append(labels);
  return node;
}

/** Desktop reviews use the same card, option, preview and navigation styles as
 * the other agents. Cursor's IDs and decisions remain local to its protocol. */
export function createCursorReview(request: CursorRequest, send: (decision: CursorDecision) => void, back?: () => void) {
  const form = element("form", "cursor-review-form");
  form.addEventListener("submit", event => event.preventDefault());
  const status = request.body.kind === "plan" ? element("p", "permission-submit-status") : undefined;
  if (status) {
    status.setAttribute("role", "status");
    status.hidden = true;
  }
  const error = element("p", "permission-submit-status");
  error.dataset.state = "error";
  error.setAttribute("role", "alert");
  let editable = false;
  let syncQuestion = () => {};
  const previews: { preview: HTMLElement; summary: HTMLElement; popover: HTMLElement }[] = [];

  const preview = (summary: HTMLElement, full: HTMLElement, label: string, plan = false) => {
    const node = element("div", `question-preview${plan ? " plan-preview" : ""}`);
    const popover = element("aside", `question-popover${plan ? " plan-popover" : ""}`);
    popover.id = `cursor-${request.requestId}-preview-${previews.length}`;
    popover.setAttribute("role", "tooltip");
    popover.append(element("span", "question-popover__label", t(label)), full);
    node.append(summary, popover);
    previews.push({ preview: node, summary, popover });
    return node;
  };
  const actionsBlock = () => {
    const block = element("section", "question-block question-block--answers");
    block.setAttribute("aria-label", t("确认选项"));
    block.append(header(t("操作"), t("点击后立即回传")));
    return block;
  };
  const feedbackStatus = () => {
    // Tool and question cards keep only actionable validation/submission errors.
    if (!status) {
      form.append(error);
      return;
    }
    const recorded = element("pre", "cursor-recorded permission-submit-status");
    recorded.hidden = true;
    const notice = element("p", "permission-cwd", t(request.origin === "hook"
      ? "CodeCraft Hook 审批；Cursor 仍可能要求独立确认。"
      : "CodeCraft 独立工作流；本次审阅不会接管 Cursor 原生审批。"));
    notice.title = notice.textContent ?? "";
    const footer = element("div", "cursor-review-status");
    footer.append(error, status, recorded, notice);
    form.append(footer);
  };

  if (request.body.kind === "permission") {
    const content = request.body.content;
    const prompt = element("section", "question-block question-block--prompt");
    const { summary, details } = cursorToolDisplay(content, t);
    prompt.append(header(t("工具调用"), content.toolName, back),
      preview(element("p", "cursor-tool-input", summary), element("p", "cursor-tool-input", summary), "工具调用"));
    if (details !== null) prompt.append(createCursorToolDetails(details, t("完整工具参数")));
    const block = actionsBlock();
    const options = element("div", "permission-options");
    options.setAttribute("role", "group");
    options.setAttribute("aria-label", t("确认操作"));
    options.append(
      option(t("允许一次"), t("仅执行这一次，不保存规则"), "permission-option permission-option--allow",
        () => send({ kind: "permission", action: "allowOnce" })),
      option(t("拒绝"), t("不执行这个工具并继续"), "permission-option permission-option--deny",
        () => send({ kind: "permission", action: "deny" })),
    );
    block.append(options);
    form.append(prompt, block);
    feedbackStatus();
  } else if (request.body.kind === "question") {
    const questions = request.body.content.questions;
    const answers: CursorAnswer[] = questions.map(q => ({ questionId: q.questionId, selectedOptionIds: [], text: null, skipped: false }));
    let index = 0;
    const pages: HTMLElement[] = [];
    const syncPages: (() => void)[] = [];
    questions.forEach((question, questionIndex) => {
      const answer = answers[questionIndex];
      const page = element("div", "cursor-question-page");
      const prompt = element("section", "question-block question-block--prompt");
      prompt.append(header(t(`第 ${questionIndex + 1}/${questions.length} 问题`), "Cursor", back),
        preview(element("p", "", question.prompt), element("p", "", question.prompt), "完整问题"));
      const block = element("section", "question-block question-block--answers");
      block.dataset.collapsed = "false";
      const options = element("div", "question-options");
      options.id = `cursor-${request.requestId}-options-${questionIndex}`;
      options.setAttribute("role", "group");
      options.setAttribute("aria-label", question.prompt);
      const toggle = action("", "question-answer-toggle", () => {
        const collapsed = block.dataset.collapsed !== "true";
        block.dataset.collapsed = String(collapsed);
        toggle.setAttribute("aria-expanded", String(!collapsed));
        toggle.setAttribute("aria-label", t(collapsed ? "展开回答选项" : "收起回答选项"));
      }, false);
      toggle.setAttribute("aria-controls", options.id);
      toggle.setAttribute("aria-expanded", "true");
      toggle.setAttribute("aria-label", t("收起回答选项"));
      toggle.append(element("span", "answer-mode", t(question.multiSelect ? "可多选" : "单选")), icon("m3 7 5-4 5 4", "0 0 16 10"));
      let extraOpen = question.options.length === 0;
      const extra = element("section", "question-block question-block--extra");
      const free = element("textarea");
      free.rows = 2;
      free.placeholder = t("输入你的回答或补充说明");
      free.setAttribute("aria-label", t("补充回答"));
      free.dataset.cursorWrite = "true";
      free.value = "";
      free.addEventListener("input", () => {
        answer.text = free.value || null;
        answer.skipped = false;
        const validation = validateCursorAnswers([question], [answer]);
        error.textContent = answer.text && validation ? t(validation) : "";
        free.setAttribute("aria-invalid", String(Boolean(answer.text && validation)));
        syncQuestion();
      });
      extra.append(free);
      const choices = question.options.map(choice => {
        const button = option(choice.label, "", "", () => {
          answer.skipped = false;
          answer.selectedOptionIds = question.multiSelect
            ? answer.selectedOptionIds.includes(choice.optionId)
              ? answer.selectedOptionIds.filter(id => id !== choice.optionId)
              : [...answer.selectedOptionIds, choice.optionId]
            : [choice.optionId];
          error.textContent = "";
          syncQuestion();
        });
        options.append(button);
        return button;
      });
      let other: HTMLButtonElement | undefined;
      if (question.allowFreeText && question.options.length) {
        other = option(t("其他"), t("输入你的回答或补充说明"), "", () => {
          extraOpen = !extraOpen;
          answer.skipped = false;
          if (!extraOpen) { answer.text = null; free.value = ""; }
          error.textContent = "";
          syncQuestion();
          if (extraOpen) free.focus();
        });
        options.append(other);
      }
      let skip: HTMLButtonElement | undefined;
      if (question.allowSkip) {
        skip = option(t("跳过此题"), "", "", () => {
          answer.skipped = !answer.skipped;
          if (answer.skipped) {
            answer.selectedOptionIds = [];
            answer.text = null;
            free.value = "";
            extraOpen = false;
          }
          error.textContent = "";
          syncQuestion();
        });
        options.append(skip);
      }
      block.append(toggle, options);
      block.hidden = !options.children.length;
      page.append(prompt, block, extra);
      pages.push(page);
      form.append(page);
      syncPages.push(() => {
        choices.forEach((button, choiceIndex) => button.setAttribute("aria-pressed", String(answer.selectedOptionIds.includes(question.options[choiceIndex].optionId))));
        other?.setAttribute("aria-pressed", String(extraOpen));
        skip?.setAttribute("aria-pressed", String(answer.skipped));
        extra.hidden = !question.allowFreeText || answer.skipped || (question.options.length > 0 && !extraOpen);
      });
    });
    const navigation = element("nav", "question-actions");
    navigation.setAttribute("aria-label", t("问题导航"));
    const move = (nextIndex: number) => {
      index = nextIndex;
      error.textContent = "";
      syncQuestion();
      const target = pages[index].querySelector<HTMLElement>(".question-block__header h1");
      target?.setAttribute("tabindex", "-1");
      target?.focus({ preventScroll: true });
    };
    const previous = action("上一步", "question-action question-action--secondary", () => move(index - 1), false);
    const cancel = action("取消问题", "question-action question-action--secondary", () => send({ kind: "questionCancelled" }));
    const next = action("下一步", "question-action question-action--primary", () => move(index + 1), false);
    const submit = action("提交", "question-action question-action--primary", () => send({ kind: "answered", answers: answers.map(a => ({ ...a, selectedOptionIds: [...a.selectedOptionIds] })) }));
    navigation.append(previous, cancel, next, submit);
    form.append(navigation);
    feedbackStatus();
    syncQuestion = () => {
      pages.forEach((page, pageIndex) => { page.hidden = pageIndex !== index; });
      syncPages.forEach(sync => sync());
      previous.hidden = index === 0;
      next.hidden = index === questions.length - 1 || (editable && validateCursorAnswers([questions[index]], [answers[index]]) !== null);
      submit.hidden = index !== questions.length - 1;
      submit.disabled = !editable || validateCursorAnswers(questions, answers) !== null;
    };
  } else {
    const content = request.body.content;
    const prompt = element("section", "question-block question-block--prompt");
    const markdown = element("div", "plan-markdown plan-summary-text");
    markdown.append(element("h3", "", content.title));
    if (content.overview) markdown.append(element("p", "", content.overview));
    const body = element("div");
    body.innerHTML = renderMarkdown(content.markdown);
    markdown.append(body);
    for (const phase of content.phases) markdown.append(element("h4", "", phase.title), element("p", "", phase.description));
    if (content.todos.length) {
      const todos = element("ul");
      const states: Record<string, string> = { pending: "待开始", inProgress: "进行中", completed: "已完成", cancelled: "已取消" };
      for (const todo of content.todos) {
        const phase = content.phases.find(phase => phase.id === todo.phaseId)?.title ?? todo.phaseId;
        todos.append(element("li", "", `[${t(states[todo.status] ?? todo.status)}] ${todo.description}${phase ? ` · ${phase}` : ""}`));
      }
      markdown.append(todos);
    }
    const full = markdown.cloneNode(true) as HTMLElement;
    full.classList.remove("plan-summary-text");
    prompt.append(header(t("计划"), `Cursor · ${t("审阅版本")} ${request.submission.plan?.revision}`, back), preview(markdown, full, "完整计划", true));
    const block = actionsBlock();
    const options = element("div", "permission-options plan-options");
    options.setAttribute("role", "group");
    options.setAttribute("aria-label", t("实行计划操作"));
    const feedback = element("textarea", "plan-custom-input");
    feedback.rows = 1;
    feedback.maxLength = 16384;
    feedback.placeholder = t("请输入需要修改的内容");
    feedback.setAttribute("aria-label", t("审阅意见"));
    feedback.dataset.cursorWrite = "true";
    const decide = (action: "approved" | "revise" | "rejected" | "cancelled") => send({ kind: "plan", action, feedback: feedback.value });
    options.append(option(t("批准计划"), t("按此版本继续执行"), "permission-option permission-option--allow plan-execute-option", () => decide("approved")),
      feedback, action("要求修改", "question-action question-action--primary plan-custom-submit", () => decide("revise")));
    const secondary = element("nav", "question-actions cursor-plan-secondary");
    secondary.append(action("拒绝计划", "question-action question-action--secondary", () => decide("rejected")),
      action("取消审阅", "question-action question-action--secondary", () => decide("cancelled")));
    block.append(options, secondary);
    form.append(prompt, block);
    feedbackStatus();
  }

  const measure = () => {
    for (const { preview, summary, popover } of previews) {
      if (!summary.getClientRects().length) continue;
      const clipped = summary.scrollHeight > summary.clientHeight + 1;
      preview.dataset.clipped = String(clipped);
      if (clipped) {
        preview.tabIndex = 0;
        preview.setAttribute("aria-describedby", popover.id);
      } else {
        preview.removeAttribute("tabindex");
        preview.removeAttribute("aria-describedby");
      }
    }
  };
  const observer = new ResizeObserver(measure);
  previews.forEach(({ summary }) => observer.observe(summary));
  return {
    form, status, error,
    sync(allowed: boolean) {
      editable = allowed;
      form.querySelectorAll<HTMLButtonElement | HTMLTextAreaElement>("[data-cursor-write]").forEach(node => { node.disabled = !allowed; });
      syncQuestion();
    },
    dispose() { observer.disconnect(); },
  };
}
