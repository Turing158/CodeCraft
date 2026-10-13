import { currentLocale, translateText } from "./i18n";
import { renderMarkdown } from "./plan-markdown";
import { cursorRequestIdentity, cursorRequestLabel, cursorRequestStatus, validateCursorAnswers, type CursorAnswer, type CursorDecision, type CursorRequest } from "./cursor-interactions";
import type { CursorSnapshot } from "./cursor-sessions";
import { createCursorReview } from "./cursor-review";
import { cursorToolDisplay, createCursorToolDetails } from "./cursor-tool-display";
import "./cursor-interactions.css";

const t = (s: string) => translateText(s,currentLocale());
const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text?: string) => { const e = document.createElement(tag); if (text !== undefined) e.textContent = t(text); return e; };
const button = (text: string, action: () => void) => {const b=el("button",text);b.type="button";b.addEventListener("click",action);return b;};
type Submit = (route: "permission" | "question" | "plan" | "cancel", body: unknown) => Promise<unknown>;
export class CursorView {
  private snapshot?: CursorSnapshot;
  private write=false;
  private host?: HTMLElement;
  private active?: string;
  private signature="";
  private form?: HTMLFormElement;
  private status?: HTMLElement;
  private error?: HTMLElement;
  private feedback?: HTMLTextAreaElement;
  private answers: CursorAnswer[]=[];
  private busy = new Set<string>();
  private back?: () => void;
  private desktopReview?: ReturnType<typeof createCursorReview>;
  constructor(private submit: Submit, private refresh: () => Promise<void>,
    private onSubmitted?: (request: CursorRequest, decision: CursorDecision) => void) {}
  detach() {
    this.desktopReview?.dispose();this.desktopReview=undefined;
    this.host?.querySelector(":scope > .cursor-interactions")?.remove();
    this.host?.classList.remove("cursor-interaction-active");this.host=undefined;this.active=undefined;this.signature="";
    this.form=undefined;this.status=undefined;this.error=undefined;this.feedback=undefined;this.answers=[];
  }
  update(snapshot: CursorSnapshot, write: boolean) {this.snapshot=snapshot;this.write=write;this.render();}
  show(host: HTMLElement, id: string, back?: () => void) {
    if (this.host !== host) this.detach();
    this.host=host;this.active=id;this.back=back;this.render();
  }
  private allowed(r: CursorRequest) {
    const cap=this.snapshot?.capabilities;
    return this.write && this.snapshot?.connected === true && r.actionable && r.submission.appEpoch === this.snapshot.appEpoch
      && r.expiresAt > Date.now() && (r.body.kind === "permission" ? cap?.hookOnly : r.body.kind === "question" ? cap?.askUser : cap?.reviewPlan) === true;
  }
  private render() {
    if (!this.host || !this.active || !this.snapshot) return;
    const r=this.snapshot.requests.find(r=>r.requestId===this.active);
    if (!r) {
      if(this.desktopReview)this.desktopReview.sync(false);
      else this.form?.querySelectorAll<HTMLInputElement|HTMLButtonElement|HTMLTextAreaElement>("input,button,textarea").forEach(e=>e.disabled=true);
      if(this.status){this.status.textContent=t("请求已失效，请返回会话列表。");this.status.hidden=false;}return;
    }
    this.host.classList.add("cursor-interaction-active");
    const signature=cursorRequestIdentity(r);
    if (signature !== this.signature) {
      this.signature=signature;
      this.desktopReview?.dispose();this.desktopReview=undefined;
      let container=this.host.querySelector<HTMLElement>(":scope > .cursor-interactions");
      if (!container) {container=el("section");container.className="cursor-interactions";this.host.append(container);}
      if (this.host.classList.contains("panel-view")) {
        container.classList.add("cursor-interactions--desktop");
        this.desktopReview=createCursorReview(r,decision=>void this.send(decision),this.back?()=>this.back?.():undefined);
        this.form=this.desktopReview.form;this.status=this.desktopReview.status;this.error=this.desktopReview.error;
        container.replaceChildren(this.form);
      } else {
        const header=el("header");
        if (this.back) header.append(button("返回会话",()=>this.back?.()));
        header.append(el("h2",cursorRequestLabel(r)));
        const provenance=el("p",r.origin === "hook" ? "CodeCraft Hook 审批；Cursor 仍可能要求独立确认。" : "CodeCraft 独立工作流；本次审阅不会接管 Cursor 原生审批。");
        provenance.className="cursor-interactions__notice";
        this.status=el("p");this.status.setAttribute("role","status");
        this.form=el("form");this.form.addEventListener("submit",e=>e.preventDefault());
        this.feedback=undefined;this.answers=[];
        this.error=el("p");this.error.setAttribute("role","alert");this.error.className="cursor-interactions__error";
        const actions=el("div");actions.className="cursor-interactions__actions";
        if (r.body.kind === "permission") {
          this.form.append(el("h3",r.body.content.toolName));
          const {summary,details}=cursorToolDisplay(r.body.content,t);
          const description=el("p");description.className="cursor-tool-input";description.textContent=summary;this.form.append(description);
          if(details!==null)this.form.append(createCursorToolDetails(details,t("完整工具参数")));
          actions.append(button("允许本次",()=>void this.send({kind:"permission",action:"allowOnce"})),button("拒绝本次",()=>void this.send({kind:"permission",action:"deny"})));
        } else if (r.body.kind === "question") {
          for (const q of r.body.content.questions) {
            const answer:CursorAnswer={questionId:q.questionId,selectedOptionIds:[],text:null,skipped:false};this.answers.push(answer);
            const group=el("fieldset");group.append(el("legend",q.prompt+(q.required?" *":"")));
            for (const o of q.options) {
              const label=el("label");const input=el("input");input.type=q.multiSelect?"checkbox":"radio";input.name=q.questionId;input.value=o.optionId;
              input.addEventListener("change",()=>{answer.skipped=false; if(skip)skip.checked=false;
                answer.selectedOptionIds=q.multiSelect ? [...group.querySelectorAll<HTMLInputElement>('input[data-option]:checked')].map(e=>e.value) : input.checked?[o.optionId]:[];});
              input.dataset.option="true";label.append(input,el("span",o.label));group.append(label);
            }
            let free:HTMLTextAreaElement|undefined;
            if (q.allowFreeText) {
              const label=el("label","补充回答");free=el("textarea");free.rows=3;free.setAttribute("aria-label",q.prompt);
              free.addEventListener("input",()=>{answer.text=free!.value;answer.skipped=false;if(skip)skip.checked=false;});label.append(free);group.append(label);
            }
            let skip:HTMLInputElement|undefined;
            if (q.allowSkip) {
              const label=el("label","跳过此题");skip=el("input");skip.type="checkbox";
              skip.addEventListener("change",()=>{answer.skipped=skip!.checked;if(answer.skipped){answer.text=null;answer.selectedOptionIds=[];
                group.querySelectorAll<HTMLInputElement>("input[data-option]").forEach(e=>e.checked=false);if(free)free.value="";}});label.prepend(skip);group.append(label);
            }
            this.form.append(group);
          }
          actions.append(button("提交回答",()=>void this.send({kind:"answered",answers:this.answers.map(a=>({...a,selectedOptionIds:[...a.selectedOptionIds]}))})),button("取消问题",()=>void this.send({kind:"questionCancelled"})));
        } else {
          const p=r.body.content;
          this.form.append(el("h3",p.title),el("p",`${t("审阅版本")} ${r.submission.plan?.revision}`),el("p",p.overview));
          const markdown=el("div");markdown.className="cursor-interactions__markdown";markdown.innerHTML=renderMarkdown(p.markdown);this.form.append(markdown);
          if (p.phases.length) {const phases=el("section");phases.append(el("h3","阶段"));p.phases.forEach(p=>phases.append(el("h4",p.title),el("p",p.description)));this.form.append(phases);}
          if (p.todos.length) {const todos=el("ul");const status:Record<string,string>={pending:"待开始",inProgress:"进行中",completed:"已完成",cancelled:"已取消"};p.todos.forEach(todo=>todos.append(el("li",`[${t(status[todo.status]??todo.status)}] ${todo.description}${todo.phaseId?` · ${p.phases.find(p=>p.id===todo.phaseId)?.title??todo.phaseId}`:""}`)));this.form.append(todos);}
          const label=el("label","审阅意见");this.feedback=el("textarea");this.feedback.rows=3;this.feedback.setAttribute("aria-label",t("审阅意见"));label.append(this.feedback);this.form.append(label);
          for (const [label,action] of [["批准计划","approved"],["要求修改","revise"],["拒绝计划","rejected"],["取消审阅","cancelled"]] as const)
            actions.append(button(label,()=>void this.send({kind:"plan",action,feedback:this.feedback?.value??""})));
        }
        this.form.append(this.error,actions);
        container.replaceChildren(header,provenance,this.status,this.form);
      }
    }
    const allowed=this.allowed(r)&&!this.busy.has(signature);
    if(this.desktopReview)this.desktopReview.sync(allowed);
    else this.form?.querySelectorAll<HTMLInputElement|HTMLButtonElement|HTMLTextAreaElement>("input,button,textarea").forEach(e=>e.disabled=!allowed);
    if(this.status){
      this.status.textContent=this.busy.has(signature)?t("正在回传决定…"):t(cursorRequestStatus(r))+(r.actionable&&!this.write?` ${t("当前连接只读，无法提交决定。")}`:"");
      this.status.hidden=!!this.desktopReview&&allowed&&r.phase==="pending";
    }
    if (r.body.kind === "plan" && this.form) {
      let recorded=this.form.querySelector<HTMLElement>(".cursor-recorded");
      if (!recorded) {recorded=el("pre");recorded.className="cursor-recorded";this.form.append(recorded);}
      const decision=r.decision?.value;
      recorded.hidden=decision?.kind!=="plan";
      recorded.textContent=decision?.kind==="plan"
        ? t("已记录决定")+"\n"+t(({approved:"已批准计划",revise:"要求修改",rejected:"已拒绝计划",cancelled:"已取消审阅"} as const)[decision.action])+"\n"+decision.feedback
        : "";
    }
  }
  private async send(decision: CursorDecision) {
    const r=this.snapshot?.requests.find(r=>r.requestId===this.active);
    if (!r || !this.allowed(r) || this.busy.has(cursorRequestIdentity(r))) {this.render();return;}
    if (decision.kind === "answered" && r.body.kind === "question") {
      const error=validateCursorAnswers(r.body.content.questions,decision.answers);if(error){if(this.error)this.error.textContent=t(error);return;}
    }
    if (decision.kind === "plan" && decision.action === "revise" && !decision.feedback.trim()) {if(this.error)this.error.textContent=t("请填写修改意见。");return;}
    const identity=cursorRequestIdentity(r);
    this.busy.add(identity);if(this.error)this.error.textContent="";this.render();
    try {
      await this.submit(r.body.kind,{schemaVersion:1,submission:r.submission,decision});
      this.onSubmitted?.(r,decision);
      await this.refresh();
    }
    catch(error) {
      if(this.signature===cursorRequestIdentity(r)&&this.error) {
        const e=error as {error?:{message?:string};message?:string};this.error.textContent=t(e.error?.message??e.message??String(error));
      }
    } finally {this.busy.delete(identity);this.render();}
  }
}
