/** Reuse handoff controls across snapshots so focus and launch state survive refreshes. */
export class WorkBuddyHandoffUi {
  private actions = new Map<string, HTMLElement>();

  constructor(private open: (requestKey: string) => Promise<void>) {}

  openRequest(requestKey: string): Promise<void> {
    return this.open(requestKey);
  }

  forRequest(requestKey: string): HTMLElement {
    const existing = this.actions.get(requestKey);
    if (existing) return existing;
    const actions = document.createElement("div");
    actions.className = "workbuddy-observation__actions";
    actions.dataset.requestKey = requestKey;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "question-action question-action--primary";
    button.textContent = "前往WorkBuddy中处理";
    const status = document.createElement("p");
    status.className = "workbuddy-observation__reason";
    status.setAttribute("role", "status");
    status.hidden = true;
    button.addEventListener("click", async () => {
      button.disabled = true;
      status.hidden = false;
      status.textContent = "正在切换到 WorkBuddy…";
      try {
        await this.openRequest(requestKey);
        status.textContent = "已切换到 WorkBuddy，处理完成后提醒会自动关闭。";
      } catch (error) {
        status.textContent = String(error);
      } finally {
        button.disabled = false;
      }
    });
    actions.append(button, status);
    this.actions.set(requestKey, actions);
    return actions;
  }

  prune(requestKeys: Set<string>) {
    for (const key of this.actions.keys()) {
      if (!requestKeys.has(key)) this.actions.delete(key);
    }
  }
}
