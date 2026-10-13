import { emptyCursorSnapshot, type CursorSnapshot } from "./cursor-sessions";
import type { CursorDecision, CursorRequest } from "./cursor-interactions";

/** Local UI fixture only. It never calls Tauri, installs files or grants a capability. */
export const isCursorPreview = () => import.meta.env.DEV && typeof window !== "undefined"
  && new URLSearchParams(window.location.search).get("cursor") === "1";

export function createCursorPreviewSnapshot(): CursorSnapshot {
  if (typeof window !== "undefined" && new URLSearchParams(window.location.search).get("interactions") === "1") {
    interactive ??= createCursorInteractionPreview();return interactive;
  }
  const now = Date.now();
  return { ...emptyCursorSnapshot(), appEpoch: "synthetic-preview", version: 1, connected: true, lastObservationAt: now,
    hook: { enabled: true, filesInstalled: true, detected: true, cursorVersion: "3.21.16", cursorExecutable: "D:/Cursor.exe", installPath: "D:/.cursor", configRevision: 1, state: "installed", error: null },
    sessions: [{ source: "cursor", key: "a".repeat(64), conversationId: "synthetic-session", title: "Cursor 采集预览 · 合成数据", updatedAt: now, activity: "interrupted", hasCurrentRequest: false, readOnly: true, truncated: false,
      items: [{ kind: "tool", generation: "g", callId: "tool-1", name: "Read", outcome: "started" },
        { kind: "response", generation: "g", text: "这是界面验证用的合成记录。重启后，未确认的工具结果保持未知。", truncated: false }] }] };
}

let interactive:CursorSnapshot|undefined;
export function createCursorInteractionPreview():CursorSnapshot {
  const now=Date.now(),epoch="synthetic-interactions";
  const requests:CursorRequest[]=(["question","plan","permission"] as const).map((kind,index)=>{
    const key=String(index+1).repeat(64),id=`synthetic-${kind}`;
    return {requestId:id,sessionKey:key,workflowId:kind==="permission"?null:`workflow-${index}`,origin:kind==="permission"?"hook":"independentMcp",
      title:`Cursor ${kind} · 合成演示`,createdAt:now+index,updatedAt:now+index,expiresAt:now+3600000,actionable:true,phase:"pending",transport:"notStarted",reason:null,decision:null,
      submission:{requestId:id,appEpoch:epoch,requestVersion:2,argumentsDigest:key,plan:kind==="plan"?{planId:"demo-plan",revision:1,contentDigest:"f".repeat(64)}:null},
      body:kind==="question"?{kind,content:{questions:[
        {questionId:"target",prompt:"合成问题：选择目标环境",options:[{optionId:"dev",label:"开发环境"},{optionId:"test",label:"测试环境"}],multiSelect:false,allowFreeText:true,required:true,allowSkip:false,maxTextLength:100},
        {questionId:"checks",prompt:"合成问题：选择检查项",options:[{optionId:"unit",label:"单元测试"},{optionId:"build",label:"构建"}],multiSelect:true,allowFreeText:false,required:false,allowSkip:true,maxTextLength:0}
      ]}}:kind==="plan"?{kind,content:{title:"合成计划：验证独立审批",overview:"此演示不会执行命令或修改配置。",markdown:"## 实施步骤\n\n1. 校验输入与请求版本。\n2. 在桌面和 LAN 完成人工审阅。\n3. 验证取消、替换和连接中断。\n\n`<script>` 会按普通文字显示。",todos:[{id:"t1",description:"验证重复提交与相反决定",status:"pending",phaseId:"p1"}],phases:[{id:"p1",title:"验证阶段",description:"共享后端接受唯一决定。"}]}}:
      {kind,content:{eventName:"preToolUse",toolName:"Shell · 合成演示",toolInput:{command:"echo synthetic-preview"}}}};
  });
  return {...emptyCursorSnapshot(),interactionProtocol:1,appEpoch:epoch,version:1,connected:true,lastObservationAt:now,
    capabilities:{hookOnly:true,nativeShared:false,askUser:true,reviewPlan:true,reason:"合成演示，不会连接 Cursor。",blockers:[],interactionMode:"independent"},
    hook:{enabled:true,filesInstalled:true,detected:true,cursorVersion:"合成演示",cursorExecutable:null,installPath:"synthetic",configRevision:1,state:"installed",error:null},requests,
    sessions:requests.map(r=>({key:r.sessionKey,source:"cursor",conversationId:r.requestId,title:r.title,updatedAt:r.updatedAt,activity:"working",items:[],truncated:false,hasCurrentRequest:true,readOnly:true,interactionOrigin:r.origin==="hook"?"hook":"independentMcp"}))};
}
export function applyCursorPreviewDecision(body:unknown) {
  if (!isCursorPreview() || !interactive) throw new Error("Preview is inactive");
  const value=body as {submission:CursorRequest["submission"];decision:CursorDecision};
  const request=interactive.requests.find(r=>r.requestId===value.submission?.requestId);
  if(!request?.actionable||JSON.stringify(request.submission)!==JSON.stringify(value.submission))throw new Error("STALE_REQUEST");
  interactive={...interactive,version:interactive.version+1,requests:interactive.requests.map(r=>r===request?{...r,actionable:false,phase:"decided",transport:"written",decision:{value:value.decision}}:r),
    sessions:interactive.sessions.map(s=>s.key===request.sessionKey?{...s,hasCurrentRequest:false,activity:"stopped"}:s)};
}
