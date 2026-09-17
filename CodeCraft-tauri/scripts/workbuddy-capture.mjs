/** Real WorkBuddy-bundled CLI with a deterministic local model transport.
 * No account, cloud model, user settings, or pre-existing session is used.
 * These captures prove runtime behavior, not WorkBuddy Desktop UI behavior.
 */
import fs from "node:fs/promises";
import path from "node:path";
import http from "node:http";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runtime = process.env.WORKBUDDY_CLI_ENTRY || "D:/software/WorkBuddy/resources/app.asar.unpacked/cli/bin/codebuddy";
const bundle = path.resolve(path.dirname(runtime), "../dist/codebuddy-headless.js");
const provenance = {
  capturedAt: new Date().toISOString(),
  entrySha256: crypto.createHash("sha256").update(await fs.readFile(runtime)).digest("hex"),
  bundleSha256: crypto.createHash("sha256").update(await fs.readFile(bundle)).digest("hex"),
};
const scenario = process.argv[2] || "tool-allow";
const base = path.join(project, ".workbuddy-verification");
await fs.mkdir(base, {recursive:true});
const directory = await fs.mkdtemp(path.join(base, `${scenario}-`));
await fs.mkdir(path.join(directory, "workspace"));
const workspace = path.join(directory, "workspace");
await fs.writeFile(path.join(workspace, "input.txt"), "CodeCraft isolated fixture input.\n", {flag:"wx"});
const question = {questions:[{header:"Mode",question:"Which mode?",options:[{label:"Read",description:"Observe"},{label:"Full",description:"Verify"}],multiSelect:false}]};
const cases = {
  "tool-allow": {tool:"Write",args:{file_path:path.join(workspace,"allowed.txt"),content:"fixture allow\n"},decision:"allow"},
  "tool-deny": {tool:"Write",args:{file_path:path.join(workspace,"denied.txt"),content:"must not be written\n"},decision:"deny"},
  "tool-ask": {tool:"Write",args:{file_path:path.join(workspace,"ask.txt"),content:"must wait for native approval\n"},decision:"ask"},
  "read": {tool:"Read",args:{file_path:path.join(workspace,"input.txt")},decision:"allow"},
  "failure": {tool:"Read",args:{file_path:path.join(workspace,"missing.txt")},decision:"allow"},
  "question-single": {tool:"AskUserQuestion",args:question,decision:"allow",modifiedInput:{answers:{"Which mode?":"Full"}}},
  "question-multiple": {tool:"AskUserQuestion",args:{questions:[{...question.questions[0],multiSelect:true}]},decision:"allow",modifiedInput:{answers:{"Which mode?":"Read, Full"}}},
  "question-text": {tool:"AskUserQuestion",args:question,decision:"allow",modifiedInput:{answers:{"Which mode?":"Custom fixture answer"}}},
  "plan-allow": {tool:"ExitPlanMode",args:{allowedPrompts:[]},planText:"# Isolated plan\n1. Read input.txt.\n2. Do not modify existing files.",decision:"allow",plan:true},
  "plan-deny": {tool:"ExitPlanMode",args:{allowedPrompts:[]},planText:"# Isolated plan\n1. Read input.txt.",decision:"deny",plan:true},
  "timeout": {tool:"Write",args:{file_path:path.join(workspace,"timeout.txt"),content:"must not be written\n"},delayMs:2000,hookTimeout:1},
  "fallback": {tool:"Write",args:{file_path:path.join(workspace,"fallback.txt"),content:"requires native approval\n"}},
};
const test = cases[scenario];
if (!test) throw new Error(`Unknown scenario: ${scenario}`);
await fs.writeFile(path.join(directory,"policy.json"), JSON.stringify(test,null,2), {flag:"wx"});
const hook = `"${process.execPath.replaceAll("\\","/")}" "${path.join(project,"scripts/workbuddy-fixture-hook.mjs").replaceAll("\\","/")}"`;
const events = ["SessionStart","SessionEnd","UserPromptSubmit","PreToolUse","PostToolUse","PostToolUseFailure","Stop","PermissionRequest","PermissionDenied"];
const settings = {disableAllHooks:false,autoUpdates:false,autoCompactEnabled:false,hooks:Object.fromEntries(events.map((event)=>[event,[{matcher:"*",hooks:[{type:"command",command:hook,timeout:event==="PreToolUse"?(test.hookTimeout||10):10}]}]]))};
await fs.writeFile(path.join(directory,"settings.json"),JSON.stringify(settings,null,2),{flag:"wx"});
const messages = [];
let toolSent = false;
const server = http.createServer(async(req,res)=>{
  const chunks=[];
  for await(const c of req) chunks.push(c);
  let input;
  try { input=JSON.parse(Buffer.concat(chunks).toString("utf8")); } catch { input={}; }
  if (!req.url?.includes("chat/completions")) {res.writeHead(200,{"content-type":"application/json"});res.end(JSON.stringify({data:[],ok:true}));return;}
  messages.push({path:req.url,messages:input.messages,toolNames:input.tools?.map((t)=>t.function?.name),tools:input.tools?.filter((t)=>[test.tool,"EnterPlanMode"].includes(t.function?.name))});
  const isMain=Array.isArray(input.tools)&&(input.tools.some((t)=>t.function?.name===test.tool) || test.plan && input.tools.length>0 && JSON.stringify(input.messages).includes("Run the single isolated CodeCraft protocol fixture tool call"));
  const call = isMain && !toolSent;
  let nextTool = test.tool, nextArgs = test.args;
  if(call && test.plan){
    const texts=(input.messages||[]).flatMap((m)=>typeof m.content==="string"?[m.content]:(m.content||[]).map((p)=>p.text||""));
    const target=texts.map((text)=>/create your plan at (.+?\.md) using the Write tool/.exec(text)?.[1]).find(Boolean);
    if(target){
      const resolved=path.resolve(target);
      if(!resolved.toLowerCase().startsWith(path.join(directory,"config","plans").toLowerCase()+path.sep))throw new Error("Plan fixture path escaped isolated config");
      await fs.mkdir(path.dirname(resolved),{recursive:true});
      await fs.writeFile(resolved,test.planText,{flag:"wx"});
    }
  }
  if(call && nextTool===test.tool) toolSent=true;
  const id=`chatcmpl-fixture-${messages.length}`;
  const toolCall={id:nextTool===test.tool?"call_codecraft_fixture_001":"call_codecraft_plan_write",type:"function",function:{name:nextTool,arguments:JSON.stringify(nextArgs)}};
  if(input.stream){
    res.writeHead(200,{"content-type":"text/event-stream","cache-control":"no-cache"});
    const send=(delta,finish_reason=null)=>res.write(`data: ${JSON.stringify({id,object:"chat.completion.chunk",created:Math.floor(Date.now()/1000),model:input.model,choices:[{index:0,delta,finish_reason}]})}\n\n`);
    send({role:"assistant",...(call?{tool_calls:[{index:0,...toolCall}]}:{content:"CodeCraft fixture completed."})});
    send({},call?"tool_calls":"stop"); res.end("data: [DONE]\n\n");
  }else{
    res.writeHead(200,{"content-type":"application/json"});res.end(JSON.stringify({id,object:"chat.completion",created:Math.floor(Date.now()/1000),model:input.model,choices:[{index:0,message:{role:"assistant",content:call?null:"CodeCraft fixture completed.",...(call?{tool_calls:[toolCall]}:{})},finish_reason:call?"tool_calls":"stop"}],usage:{prompt_tokens:10,completion_tokens:10,total_tokens:20}}));
  }
});
await new Promise((resolve)=>server.listen(0,"127.0.0.1",resolve));
const endpoint=`http://127.0.0.1:${server.address().port}/v1`;
const env={...process.env,CODEBUDDY_CONFIG_DIR:path.join(directory,"config"),CODEBUDDY_DISABLE_COMPILE_CACHE:"1",CODEBUDDY_CODE_DEBUG_LOGS_DIR:path.join(directory,"debug"),CODEBUDDY_BASE_URL:endpoint,CODEBUDDY_API_KEY:"codecraft-local-fixture-only",CODEBUDDY_MODEL:"fixture-model",CODEBUDDY_SMALL_FAST_MODEL:"fixture-model",CODEBUDDY_BIG_SLOW_MODEL:"fixture-model",DISABLE_TELEMETRY:"1",DISABLE_ERROR_REPORTING:"1",DISABLE_AUTOUPDATER:"1",CODECRAFT_CAPTURE_DIR:directory};
delete env.CODEBUDDY_AUTH_TOKEN;
const args=[runtime,"--print","--output-format","stream-json","--agent","cli","--model","fixture-model","--no-session-persistence","--setting-sources","local","--settings",path.join(directory,"settings.json"),"--strict-mcp-config","--mcp-config",'{"mcpServers":{}}',"--tools",test.tool,"--permission-mode",test.plan?"plan":"default","--max-turns","3","--session-id",`codecraft-${crypto.randomUUID()}`,"Run the single isolated CodeCraft protocol fixture tool call, then stop."];
const acp = test.tool === "AskUserQuestion" || test.plan || process.env.CODECRAFT_CAPTURE_TRANSPORT === "acp";
const childArgs=acp?[runtime,"--acp",...args.slice(4,-1)]:args;
if(test.plan) childArgs[childArgs.indexOf("--tools")+1]="default";
const child=spawn(process.execPath,childArgs,{cwd:workspace,env,windowsHide:true,stdio:["pipe","pipe","pipe"]});
let stdout="",stderr="",timedOut=false;
let lineBuffer="",rpcId=0,completed=false;
const pending=new Map();
const nativeRequests=[];
const nativeResponses=[];
const rpc=(method,params)=>new Promise((resolve,reject)=>{const id=++rpcId;pending.set(id,{resolve,reject});child.stdin.write(JSON.stringify({jsonrpc:"2.0",id,method,params})+"\n");});
child.stdout.on("data",(c)=>{
  stdout+=c;if(stdout.length>4*1024*1024)child.kill();
  if(!acp)return;
  lineBuffer+=c;
  while(lineBuffer.includes("\n")){
    const end=lineBuffer.indexOf("\n"),line=lineBuffer.slice(0,end);lineBuffer=lineBuffer.slice(end+1);
    let message;try{message=JSON.parse(line);}catch{continue;}
    if(message.method && message.id!==undefined){
      nativeRequests.push(message);
      const denied=message.params?.options?.find((o)=>o.kind==="reject_once");
      const result=message.method==="session/request_permission"?{outcome:denied?{outcome:"selected",optionId:denied.optionId}:{outcome:"cancelled"}}:{outcome:"cancelled"};
      nativeResponses.push({id:message.id,method:message.method,result});
      child.stdin.write(JSON.stringify({jsonrpc:"2.0",id:message.id,result})+"\n");
    }else if(pending.has(message.id)){
      const p=pending.get(message.id);pending.delete(message.id);message.error?p.reject(new Error(JSON.stringify(message.error))):p.resolve(message.result);
    }
  }
});
child.stderr.on("data",(c)=>{stderr+=c;if(stderr.length>1024*1024)child.kill();});
const deadline=setTimeout(()=>{timedOut=true;child.kill();},45000);
if(acp){
  (async()=>{
    await rpc("initialize",{protocolVersion:1,clientInfo:{name:"CodeCraft isolated verifier",version:"1.0"},clientCapabilities:{fs:{readTextFile:false,writeTextFile:false},terminal:false,_meta:{"codebuddy.ai":{question:true}}}});
    const session=await rpc("session/new",{cwd:workspace,mcpServers:[]});
    if(test.plan)await rpc("session/set_mode",{sessionId:session.sessionId,modeId:"plan"});
    await rpc("session/prompt",{sessionId:session.sessionId,prompt:[{type:"text",text:"Run the single isolated CodeCraft protocol fixture tool call, then stop."}]});
    completed=true;
    child.stdin.end();
    setTimeout(()=>child.kill(),1000).unref();
  })().catch((e)=>{stderr+=String(e);child.kill();});
}else child.stdin.end();
const exitCode=await new Promise((resolve,reject)=>{child.on("error",reject);child.on("exit",resolve);});
clearTimeout(deadline);server.closeAllConnections();await new Promise((resolve)=>server.close(resolve));
const report={scenario,runtime,provenance,transport:acp?"acp":"print",exitCode,completed:acp?completed:exitCode===0,timedOut,toolSent,nativeRequests,nativeResponses,modelRequestCount:messages.length,workspaceFiles:await fs.readdir(workspace),stdout,stderr,messages};
await fs.writeFile(path.join(directory,"capture.json"),JSON.stringify(report,null,2),{flag:"wx"});
const toolResults=stdout.split("\n").flatMap((line)=>{try{return JSON.parse(line).message?.content?.filter((item)=>item.type==="tool_result")||[];}catch{return[];}});
const hookFiles=(await fs.readdir(directory)).filter((n)=>n.endsWith(".hook.json"));
const hookEvents=await Promise.all(hookFiles.map(async(n)=>JSON.parse(await fs.readFile(path.join(directory,n),"utf8")).input.hook_event_name));
console.log(JSON.stringify({directory,scenario,exitCode,timedOut,toolSent,nativeRequestMethods:nativeRequests.map((m)=>m.method),modelRequestCount:messages.length,workspaceFiles:report.workspaceFiles,hookEvents,toolResults,stderr:stderr.slice(-1000)},null,2));
process.exitCode=timedOut||!report.completed?1:0;
