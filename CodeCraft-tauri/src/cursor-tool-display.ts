import { cursorRecord, type CursorRequest } from "./cursor-interactions";

type Permission = Extract<CursorRequest["body"], { kind: "permission" }>["content"];

const workflowActions: Record<string, string> = {
  codecraft_list_projects: "读取已在 CodeCraft 中启用的项目列表。",
  codecraft_start_workflow: "创建问答或计划审阅任务。",
  codecraft_ask_user: "向你提问并等待回答。",
  codecraft_review_plan: "提交计划，等待你审阅。",
  codecraft_get_result: "查询本次审阅的处理结果。",
  codecraft_cancel_request: "取消本次待处理的审阅请求。",
  codecraft_end_workflow: "结束本次审阅任务。",
};

/** Presentation only: the original arguments and approval digest stay intact. */
export function cursorToolDisplay(content: Permission, t: (text: string) => string) {
  const { toolInput, toolName, eventName } = content;
  const args = cursorRecord(toolInput) ? toolInput : undefined;
  const empty = args !== undefined && Object.keys(args).length === 0;
  const details = empty || toolInput === null || toolInput === undefined ? null
    : typeof toolInput === "string" ? toolInput : JSON.stringify(toolInput, null, 2);
  // Only describe the named CodeCraft server; a third-party tool with the same
  // short name must retain its own identity. This never grants approval.
  const ownedName = eventName === "beforeMCPExecution"
    ? /^codecraft_cursor · (?:MCP:)?(codecraft_\w+)$/.exec(toolName)?.[1] : undefined;
  const action = ownedName && Object.prototype.hasOwnProperty.call(workflowActions, ownedName) ? workflowActions[ownedName] : undefined;
  const primary = args && ["command", "file_path", "path", "url", "query"]
    .map(key => args[key]).find(value => typeof value === "string" && value.trim());
  const summary = action ? t(action)
    : typeof primary === "string" ? primary
    : `${t("调用工具")}：${toolName}${empty ? `\n${t("此工具无需参数。")}` : ""}`;
  return { summary, details };
}

export function createCursorToolDetails(details: string, label: string) {
  const container = document.createElement("details");
  container.className = "cursor-tool-details";
  const summary = document.createElement("summary");
  summary.textContent = label;
  const code = document.createElement("pre");
  code.textContent = details;
  container.append(summary, code);
  return container;
}
