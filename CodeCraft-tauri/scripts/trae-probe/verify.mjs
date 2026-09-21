// Tests the real probe executable and stdio transport. This is SYNTHETIC
// evidence, not a Trae runtime fixture, regardless of which tests pass.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

const directory=dirname(fileURLToPath(import.meta.url));
const binary=resolve(process.argv[2] ?? join(directory,'target/debug/codecraft-trae-probe.exe'));
const runs=join(directory,'runs');
await mkdir(runs,{recursive:true});
const root=join(runs,`synthetic 中文 ' ${randomUUID()}`);
const children=new Set();
const checks=[];
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function waitExit(proc) {
  if(proc.exitCode!==null)return Promise.resolve(proc.exitCode);
  return new Promise((resolve,reject)=>{
    const timeout=setTimeout(()=>{proc.kill();reject(new Error('Process did not exit after stdin EOF/error'));},8000);
    proc.once('close',code=>{clearTimeout(timeout);resolve(code);});
    proc.once('error',error=>{clearTimeout(timeout);reject(error);});
  });
}

function child(args) {
  const proc=spawn(binary,args,{windowsHide:true,stdio:['pipe','pipe','pipe']});
  children.add(proc);
  proc.once('close',()=>children.delete(proc));
  return proc;
}
async function run(args,input='',expected=0) {
  const proc=child(args); let out=''; let err='';
  proc.stdout.on('data',x=>out+=x); proc.stderr.on('data',x=>err+=x);
  proc.stdin.on('error',()=>{});
  proc.stdin.end(input);
  const timer=setTimeout(()=>proc.kill(),10000);
  const code=await new Promise((resolve,reject)=>{proc.once('close',resolve);proc.once('error',reject)});
  clearTimeout(timer);
  assert.equal(code,expected,`process exit: ${err}`);
  return out.trim()?JSON.parse(out):null;
}
const cli=(...args)=>run([...args,'--root',root]);
async function eventually(read,predicate,label) {
  const until=Date.now()+8000;
  while(Date.now()<until) {
    try {const value=await read();if(predicate(value))return value;} catch {}
    await sleep(100);
  }
  throw new Error(`Timed out: ${label}`);
}
function input(event,call=randomUUID(),session='session-a',tool='mcp__codecraft_probe__codecraft_probe_echo',args={payload:{nested:[1,true,null,'中文'],bridgeTicket:'business data'}}) {
  return {hook_event_name:event,session_id:session,cwd:join(root,'project'),workspace_roots:[join(root,'project')],tool_use_id:call,tool_name:tool,llm_tool_name:tool,tool_input:args,prompt:'P0 test prompt'};
}
const hook=(event,value)=>run(['hook','--root',root,'--event',event,'--synthetic'],JSON.stringify(value));
async function ticket(tool='codecraft_probe_echo',session='session-a',payload={nested:[1,true,null,'中文'],bridgeTicket:'business data'}) {
  const data=input('PreToolUse',randomUUID(),session,`mcp__codecraft_probe__${tool}`,{payload,bridgeTicket:'model-supplied-not-trusted'});
  const response=await hook('PreToolUse',data);
  assert.equal(response.hookSpecificOutput.permissionDecision,'allow');
  const updated=response.hookSpecificOutput.updatedInput;
  assert.deepEqual(updated.payload,payload);
  assert.match(updated.bridgeTicket,/^[A-Za-z0-9_-]{43}$/);
  return updated;
}
async function mcp() {
  const proc=child(['mcp','--root',root]);
  let next=0; let diagnostics=''; const pending=new Map(); const messages=[];
  proc.stderr.on('data',x=>diagnostics+=x);
  proc.stdin.on('error',()=>{});
  const lines=createInterface({input:proc.stdout});
  lines.on('line',line=>{
    const value=JSON.parse(line);messages.push(value);
    if(pending.has(value.id)){pending.get(value.id)(value);pending.delete(value.id);}
  });
  proc.on('close',()=>{for(const resolve of pending.values())resolve({error:{message:diagnostics||'MCP exited'}});pending.clear();});
  function send(method,params) {
    const id=++next;
    let timer;
    const result=new Promise((resolve,reject)=>{
      timer=setTimeout(()=>{pending.delete(id);reject(new Error(`MCP timeout: ${method}; ${diagnostics}`));},12000);
      pending.set(id,value=>{clearTimeout(timer);resolve(value);});
    });
    proc.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');
    return {id,result};
  }
  const initialized=await send('initialize',{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'synthetic-p0-client',version:'1'}}).result;
  assert.ok(initialized.result?.capabilities.tools,JSON.stringify(initialized));
  proc.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/initialized'})+'\n');
  return {proc,send,messages,notify:(method,params)=>proc.stdin.write(JSON.stringify({jsonrpc:'2.0',method,params})+'\n'),close:()=>{proc.stdin.end();return waitExit(proc);}};
}
const body=response=>{assert.ok(response.result,JSON.stringify(response));return response.result.structuredContent;};
async function pendingOperation(exclude=new Set()) {
  const snapshot=await eventually(()=>cli('inspect'),s=>s.operations.some(o=>o.state==='pending'&&!exclude.has(o.operationId)),'pending operation');
  return snapshot.operations.find(o=>o.state==='pending'&&!exclude.has(o.operationId)).operationId;
}

try {
  await cli('init');
  const config=JSON.parse(await readFile(join(root,'project/.trae/hooks.json'),'utf8'));
  assert.deepEqual(Object.keys(config.hooks).sort(),['SessionStart','UserPromptSubmit','PreToolUse','PostToolUse','Stop','Notification'].sort());
  assert.ok(config.hooks.PreToolUse[0].hooks[0].command.includes("''"));
  const coordinator=child(['coordinator','--root',root]);
  coordinator.stdin.end();
  await eventually(()=>cli('inspect'),s=>typeof s.appEpoch==='string','coordinator heartbeat');
  checks.push('isolated project, six fixed events and PowerShell quoting');

  const second=child(['coordinator','--root',root]);second.stdin.end();
  const secondCode=await new Promise(resolve=>second.once('close',resolve));
  assert.equal(secondCode,1);
  checks.push('exclusive OS lock rejects a second state writer');

  for(const decision of ['allow','deny','ask']) {
    await cli('case','--case',decision);
    const response=await hook('PreToolUse',input('PreToolUse'));
    assert.equal(response.hookSpecificOutput.permissionDecision,decision);
  }
  const bad=await run(['hook','--root',root,'--event','PreToolUse'],'{"x":1,"x":2}');
  assert.equal(bad.hookSpecificOutput.permissionDecision,'deny');
  const mismatch=await hook('PreToolUse',input('Stop'));
  assert.equal(mismatch.hookSpecificOutput.permissionDecision,'deny');
  const tooLarge=await run(['hook','--root',root,'--event','PreToolUse'],' '.repeat(1024*1024+1));
  assert.equal(tooLarge.hookSpecificOutput.permissionDecision,'deny');
  checks.push('allow/deny/ask encoding; duplicate keys, size and event mismatch fail closed');

  const shell=spawn('powershell.exe',['-NoProfile','-NonInteractive','-Command',config.hooks.PreToolUse[0].hooks[0].command+' --synthetic'],{windowsHide:true,stdio:['pipe','pipe','pipe']});
  children.add(shell);shell.once('close',()=>children.delete(shell));
  let shellOut='';let shellErr='';shell.stdout.on('data',x=>shellOut+=x);shell.stderr.on('data',x=>shellErr+=x);
  shell.stdin.end(JSON.stringify(input('PreToolUse')));
  assert.equal(await new Promise(resolve=>shell.once('close',resolve)),0,shellErr);
  assert.equal(JSON.parse(shellOut).hookSpecificOutput.permissionDecision,'ask');
  checks.push('generated PowerShell command executes with spaces, Unicode and apostrophes');

  await cli('case','--case','inject');
  const a=await mcp();
  const tools=await a.send('tools/list',{}).result;
  assert.equal(tools.result.tools.length,2);
  const missing=body(await a.send('tools/call',{name:'codecraft_probe_echo',arguments:{payload:1}}).result);
  assert.equal(missing.error.code,'TICKET_INVALID');
  const args=await ticket();
  const result=body(await a.send('tools/call',{name:'codecraft_probe_echo',arguments:args}).result);
  assert.equal(result.status,'completed');assert.deepEqual(result.payload,args.payload);
  const repeat=body(await a.send('tools/call',{name:'codecraft_probe_echo',arguments:args}).result);
  assert.equal(repeat.operationId,result.operationId);
  const changed=body(await a.send('tools/call',{name:'codecraft_probe_echo',arguments:{...args,payload:2}}).result);
  assert.equal(changed.error.code,'ARGUMENT_MISMATCH');
  const b=await mcp();
  const cross=body(await b.send('tools/call',{name:'codecraft_probe_echo',arguments:args}).result);
  assert.equal(cross.error.code,'TICKET_INVALID');
  assert.equal(await b.close(),0);
  checks.push('official MCP negotiation, nested ticket injection, parameter hash, same-operation retry and cross-connection rejection');

  const badMcp=await mcp();
  const badExit=waitExit(badMcp.proc);
  badMcp.proc.stdin.write('{"jsonrpc":"2.0","id":900,"method":"tools/call","params":{"name":"codecraft_probe_echo","arguments":{"payload":1,"payload":2}}}\n');
  assert.equal(await badExit,1);
  assert.ok(badMcp.messages.some(m=>m.error?.code===-32700));
  checks.push('MCP transport rejects duplicate JSON keys before SDK decoding');

  const duplicateCall=a.send('ping',{});
  a.proc.stdin.write(JSON.stringify({jsonrpc:'2.0',id:duplicateCall.id,method:'ping',params:{}})+'\n');
  await duplicateCall.result;await sleep(300);
  assert.equal(a.messages.filter(m=>m.id===duplicateCall.id).length,1);
  checks.push('duplicate JSON-RPC IDs produce only one response');

  const broken=await mcp();
  const brokenArgs=await ticket('codecraft_probe_echo','stdout-failure-session');
  const operationsBeforeBreak=new Set((await cli('inspect')).operations.map(o=>o.operationId));
  broken.proc.stdout.destroy();
  const brokenExit=waitExit(broken.proc);
  const brokenCall=broken.send('tools/call',{name:'codecraft_probe_echo',arguments:brokenArgs});
  assert.equal(await brokenExit,1);
  await brokenCall.result;
  const brokenSnapshot=await cli('inspect');
  assert.ok(brokenSnapshot.operations.some(o=>!operationsBeforeBreak.has(o.operationId)&&['delivery_prepared','failed'].includes(o.state)));
  checks.push('broken stdout exits with stdin still open and never acknowledges delivery');

  const armed=await cli('arm');
  const prompt=input('UserPromptSubmit');prompt.prompt=armed.launchPrompt;
  const bound=await hook('UserPromptSubmit',prompt);assert.ok(bound.hookSpecificOutput.additionalContext.includes('planId='));
  const duplicate=await hook('UserPromptSubmit',prompt);assert.equal(duplicate.decision,'block');
  const snapshot=await cli('inspect');assert.equal(snapshot.sessions.filter(s=>s.protected).length,1);
  const expiredTurn=body(await a.send('tools/call',{name:'codecraft_probe_echo',arguments:args}).result);assert.equal(expiredTurn.error.code,'TASK_CHANGED');
  checks.push('preallocated plan, one-session launch binding and consumed-code conservative invalidation');

  const waitArgs=await ticket('codecraft_probe_delivery');
  const first=a.send('tools/call',{name:'codecraft_probe_delivery',arguments:waitArgs});
  const op=await pendingOperation();
  const retry=a.send('tools/call',{name:'codecraft_probe_delivery',arguments:waitArgs});
  await sleep(500);
  assert.equal((await cli('inspect')).operations.filter(o=>o.operationId===op).length,1);
  a.notify('notifications/cancelled',{requestId:retry.id,reason:'cancel subscription only'});
  await sleep(400);
  assert.equal((await cli('inspect')).operations.find(o=>o.operationId===op).state,'pending');
  await cli('release','--operation',op);
  assert.equal(body(await first.result).status,'completed');
  await eventually(()=>cli('inspect'),s=>s.operations.find(o=>o.operationId===op)?.state==='delivered','flush acknowledgement');
  // The SDK may finish a cancelled request without sending a result.
  retry.result.catch(()=>{});
  checks.push('pending retries share one operation, cancelling a subscriber preserves original, flush acknowledgement commits delivery');

  await cli('transport','--before-write-ms','1200');
  const raceArgs=await ticket('codecraft_probe_delivery');
  const race=a.send('tools/call',{name:'codecraft_probe_delivery',arguments:raceArgs});
  const raceOp=await pendingOperation();
  await cli('release','--operation',raceOp);
  await eventually(()=>cli('inspect'),s=>s.operations.find(o=>o.operationId===raceOp)?.state==='delivery_prepared','prepared boundary');
  a.notify('notifications/cancelled',{requestId:race.id,reason:'cancel between handler and flush'});
  await eventually(()=>cli('inspect'),s=>s.operations.find(o=>o.operationId===raceOp)?.state==='cancelled','cancellation wins');
  race.result.catch(()=>{});
  await sleep(1400);
  assert.equal((await cli('inspect')).operations.find(o=>o.operationId===raceOp).state,'cancelled');
  checks.push('cancellation between prepare and stdout flush cannot activate delivery');

  await cli('transport','--drop-ack');
  const lostArgs=await ticket('codecraft_probe_delivery');
  const lost=a.send('tools/call',{name:'codecraft_probe_delivery',arguments:lostArgs});
  const lostOp=await pendingOperation();await cli('release','--operation',lostOp);
  assert.equal(body(await lost.result).status,'completed');
  assert.equal((await cli('inspect')).operations.find(o=>o.operationId===lostOp).state,'delivery_prepared');
  await eventually(()=>cli('inspect'),s=>s.operations.find(o=>o.operationId===lostOp)?.state==='failed','lease expiry without client polling');
  checks.push('flushed response without acknowledgement stays unconfirmed');
  assert.equal(await a.close(),0);

  const previous=(await cli('inspect')).appEpoch;
  coordinator.kill();await new Promise(resolve=>coordinator.once('close',resolve));
  const restarted=child(['coordinator','--root',root]);restarted.stdin.end();
  await eventually(()=>cli('inspect'),s=>s.appEpoch!==previous,'fresh epoch');
  const c=await mcp();
  const stale=body(await c.send('tools/call',{name:'codecraft_probe_echo',arguments:args}).result);
  assert.equal(stale.error.code,'TICKET_INVALID');assert.equal(await c.close(),0);
  checks.push('restart produces a new epoch and never restores old tickets');

  const report={evidenceKind:'synthetic',traeClientExecuted:false,checks,passed:checks.length};
  await writeFile(join(root,'synthetic-report.json'),JSON.stringify(report,null,2)+'\n');
  process.stdout.write(JSON.stringify(report,null,2)+'\n');
} finally {
  for(const proc of children)proc.kill();
}
