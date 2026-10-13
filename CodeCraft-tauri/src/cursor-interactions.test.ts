import {expect,it} from "vitest";
import {createCursorInteractionPreview} from "./cursor-preview";
import {readCursorSnapshot,CursorSnapshotTracker} from "./cursor-sessions";
import {readCursorRequests,validateCursorAnswers,type CursorAnswer} from "./cursor-interactions";
import {mergeSnapshot,canAct} from "../web/lan/snapshot";
import {cursorToolDisplay} from "./cursor-tool-display";
import {CURSOR_TRANSLATIONS} from "./cursor-i18n";

it("describes empty project lookup and workflow setup without showing raw JSON as the prompt",()=>{
  const t=(text:string)=>text;
  const lookup={eventName:"beforeMCPExecution",toolName:"codecraft_cursor · codecraft_list_projects",toolInput:{}};
  expect(cursorToolDisplay(lookup,t)).toEqual({summary:"读取已在 CodeCraft 中启用的项目列表。",details:null});
  const input={projectId:"2f50621bca15e368b1393153e04d83bdac4e7aca43ed4f27053f871e0822ac9b",title:"问答界面验证"};
  const display=cursorToolDisplay({...lookup,toolName:"codecraft_cursor · codecraft_start_workflow",toolInput:input},t);
  expect(display.summary).toBe("创建问答或计划审阅任务。");
  expect(JSON.parse(display.details!)).toEqual(input);
  expect(input.title).toBe("问答界面验证");
  expect(cursorToolDisplay(lookup,text=>CURSOR_TRANSLATIONS[text]?.[0]??text).summary).toBe("List projects enabled in CodeCraft.");
});
it("keeps commands, multiline content and third-party identities intact",()=>{
  const t=(text:string)=>text;
  const input={command:'echo "中文"',content:"第一行\n<script>alert(1)</script>",options:{dryRun:false}};
  const content={eventName:"preToolUse",toolName:"Shell",toolInput:input};
  const display=cursorToolDisplay(content,t);
  expect(display.summary).toBe(input.command);
  expect(JSON.parse(display.details!)).toEqual(input);
  const foreign=cursorToolDisplay({eventName:"beforeMCPExecution",toolName:"third_party · codecraft_list_projects",toolInput:{}},t);
  expect(foreign.summary).toBe("调用工具：third_party · codecraft_list_projects\n此工具无需参数。");
  const corrupted={title:"闂瓟鐣岄潰楠岃瘉"};
  expect(JSON.parse(cursorToolDisplay({...content,toolInput:corrupted},t).details!)).toEqual(corrupted);
});

it("retains known v5 forms through repeated desktop/LAN parsing without enabling native sync",()=>{
  const raw=createCursorInteractionPreview();
  const once=readCursorSnapshot(raw),twice=readCursorSnapshot(once);
  expect(twice.requests).toHaveLength(3);expect(twice.capabilities.nativeShared).toBe(false);
  const lan=mergeSnapshot({cursor:twice,allowApprovals:false});expect(lan.entries.every(e=>e.pending?.readOnly)).toBe(true);
  expect(lan.entries.every(e=>!canAct(e.pending,lan.allowApprovals))).toBe(true);
  const online=mergeSnapshot({cursor:twice,allowApprovals:true});expect(online.entries.every(e=>canAct(e.pending,true))).toBe(true);
});
it("rejects stale epochs, future protocol, duplicate requests and incomplete plans",()=>{
  const s=createCursorInteractionPreview();
  expect(readCursorSnapshot({...s,interactionProtocol:2}).requests).toEqual([]);
  expect(readCursorRequests([...s.requests,s.requests[0]],s.appEpoch)).toHaveLength(3);
  expect(readCursorRequests(s.requests,"different-epoch")).toEqual([]);
  expect(readCursorRequests([{...s.requests[1],submission:{...s.requests[1].submission,plan:null}}],s.appEpoch)).toEqual([]);
  const tracker=new CursorSnapshotTracker();tracker.accept(s);tracker.accept({...s,appEpoch:"new",version:0,requests:[]});
  expect(tracker.accept({...s,version:99}).appEpoch).toBe("new");
});
it("answers use IDs and validate required text, multiple selection and explicit skip",()=>{
  const r=createCursorInteractionPreview().requests[0];if(r.body.kind!=="question")throw Error();
  const questions=r.body.content.questions;
  const answers:CursorAnswer[]=[{questionId:"target",selectedOptionIds:["dev"],text:null,skipped:false},{questionId:"checks",selectedOptionIds:["unit","build"],text:null,skipped:false}];
  expect(validateCursorAnswers(questions,answers)).toBeNull();
  expect(validateCursorAnswers(questions,[{...answers[0],selectedOptionIds:[]},answers[1]])).not.toBeNull();
  expect(validateCursorAnswers(questions,[{...answers[0],selectedOptionIds:["dev","test"]},answers[1]])).not.toBeNull();
  expect(validateCursorAnswers(questions,[answers[0],{...answers[1],selectedOptionIds:[],skipped:true}])).toBeNull();
  expect(validateCursorAnswers(questions,[{...answers[0],text:"😀".repeat(101)},answers[1]])).not.toBeNull();
  expect(validateCursorAnswers(questions,[answers[0],answers[0]])).not.toBeNull();
});
