import { renderMarkdown } from "./plan-markdown";
import { currentLocale, translateText } from "./i18n";
import { canDecide, emptyTraeSnapshot, traeReviews, traeQuestionRequest, traePlanRequest, formatTraeToolArguments, TraeReviewQueue, TraeDecisionIds, type TraeReview, type TraeSnapshot } from "./trae-sessions";
import "./trae.css";

export type TraeTransport = (route: string, body: unknown) => Promise<unknown>;
const t = (s: string) => translateText(s, currentLocale());
function el<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string): HTMLElementTagNameMap[K] { const e = document.createElement(tag); if (text) e.textContent = t(text); return e; }
function button(label: string, run: () => void, disabled = false) { const b = el("button", label); b.type = "button"; b.disabled = disabled; b.addEventListener("click", run); return b; }
/** LAN/native-observation preview. Desktop uses the existing shared review pages. */
export class TraeView {
  readonly dialog = el("dialog");
  private content = el("div");
  private status = el("p");
  private title = el("strong", "Trae CN");
  private snapshot = emptyTraeSnapshot();
  private queue = new TraeReviewQueue();
  private ids = new TraeDecisionIds();
  private current?: TraeReview;
  private writable = true;
  private busy = false;
  private signature = "";
  constructor(private send: TraeTransport, private refresh: () => Promise<void>, private focus?: () => Promise<unknown>) {
    this.dialog.className = "trae-dialog";
    const header = el("header"); header.append(this.title, button("关闭", () => this.dialog.close()));
    this.status.setAttribute("role", "status");
    this.dialog.append(header, this.status, this.content); document.body.append(this.dialog);
    this.dialog.addEventListener("close", () => { this.current = undefined; this.signature = ""; });
  }
  open(sessionKey?: string) {
    const next = traeReviews(this.snapshot).find(r => !sessionKey || r.sessionKey === sessionKey);
    if (next) this.show(next);
  }
  update(snapshot: TraeSnapshot, writable = true) {
    if (snapshot.appEpoch !== this.snapshot.appEpoch) this.ids.clear();
    this.snapshot = snapshot; this.writable = writable;
    const reviews = traeReviews(snapshot);
    const next = this.queue.next(reviews, this.dialog.open ? this.current?.key : undefined);
    if (!next) { if (this.dialog.open) this.dialog.close(); return; }
    if (!this.busy) this.show(next);
  }
  private show(review: TraeReview) {
    const signature = JSON.stringify([review, this.writable, this.snapshot.capabilities, this.snapshot.connected]);
    this.current = review; this.queue.mark(review.key);
    if (signature !== this.signature) { this.signature = signature; this.status.textContent = ""; this.render(); }
    if (!this.dialog.open) this.dialog.showModal();
  }
  private render() {
    const review = this.current; if (!review) return;
    this.content.replaceChildren();
    this.title.textContent = t(review.kind === "question" ? "Trae 问题" : review.kind === "plan" ? "Trae 计划" : "Trae 工具审批");
    this.dialog.setAttribute("aria-label", this.title.textContent);
    const body = el("section"); body.className = `trae-${review.kind}-view`;
    const actions = el("footer"); actions.className = "trae-actions";
    if (review.kind === "permission") {
      const r = review.request!;
      body.append(el("h2", r.toolName), el("pre", formatTraeToolArguments(r.arguments)));
      body.append(el("p", r.state === "pending" ? "CodeCraft 审批后，Trae 仍可能要求原生确认。" : "决定已保存，等待交付"));
      if (!this.writable) body.append(el("p", "当前为只读访问"));
      for (const [label, decision] of [["允许一次", "allow"], ["拒绝", "deny"], ["交回 Trae 确认", "ask"]] as const)
        actions.append(button(label, () => void this.decide(decision), this.busy || !this.writable || !canDecide(this.snapshot, r)));
    } else {
      body.append(el("p", "仅供查看，请前往 Trae 中处理"));
      if (review.kind === "question") {
        for (const q of traeQuestionRequest(review).questions) {
          const group = el("section"); group.append(el("h2", q.question));
          const options = el("ul"); for (const option of q.options) options.append(el("li", [option.label, option.description].filter(Boolean).join(" — ")));
          group.append(options); body.append(group);
        }
      } else {
        const plan = el("div"); plan.className = "trae-markdown"; plan.innerHTML = renderMarkdown(traePlanRequest(review).plan); body.append(plan);
      }
      actions.append(button("前往 Trae 中处理", () => void this.handoff()));
    }
    this.content.append(body, actions);
  }
  private async handoff() {
    try {
      if (this.focus) await this.focus();
      else this.status.textContent = t("请在运行 Trae 的设备上打开 Trae，或使用 CodeCraft 桌面端切换到 Trae 中处理。");
    } catch (error) { this.status.textContent = String(error); }
  }
  private async decide(decision: "allow" | "deny" | "ask") {
    const r = this.current?.request;
    if (!r || this.busy || !this.writable || !canDecide(this.snapshot, r)) return;
    const action = { kind: "permission" as const, decision, message: null };
    this.busy = true; this.render();
    try {
      await this.send("permission", { schemaVersion: 1, decisionId: this.ids.for(r, action), target: r.target, action });
      this.dialog.close(); await this.refresh();
    } catch (error) {
      const e = error as { error?: { message?: string }; message?: string };
      this.status.textContent = e.error?.message ?? e.message ?? String(error);
    } finally { this.busy = false; this.render(); }
  }
}
