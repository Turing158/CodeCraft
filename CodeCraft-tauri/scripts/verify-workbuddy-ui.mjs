// Optional browser smoke test against a local Vite dev server. Test access is
// injected into the served module only; production code has no debug hook.
import assert from "node:assert/strict";
import path from "node:path";
import fs from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const modulePath = process.env.PLAYWRIGHT_MODULE;
const { chromium } = await import(modulePath ? pathToFileURL(modulePath).href : "playwright");
const base = process.env.CODECRAFT_UI_TEST_URL || "http://127.0.0.1:1421";
if (new URL(base).hostname !== "127.0.0.1") throw new Error("Local dev server required");
const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE });
const output = path.join(project, ".workbuddy-verification/ui");
await fs.mkdir(output, { recursive: true });
try {
  const page = await browser.newPage({ viewport: { width: 640, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/src/main.ts*", async (route) => {
    const response = await route.fetch();
    const body = await response.text();
    await route.fulfill({ response, body: body + `\n;globalThis.codecraftWorkBuddyTest = {
      async hook(hook, snapshot) {
        await startupSequencePromise;
        controller.setCollapseDelay(60000);
        await controller.pointerEntered();
        const status = browserHookIntegrations.find(s => s.id === "workBuddy");
        Object.assign(status, { workbuddy: hook, hookInstalled: hook.filesInstalled, installState: hook.state, error: hook.error });
        renderHookIntegrations(browserHookIntegrations.map(s => ({...s})), false);
        renderWorkBuddySnapshot({...snapshot, hook});
        syncPanelShape();
      },
      async show(snapshot) {
        await startupSequencePromise;
        controller.setCollapseDelay(60000);
        await controller.pointerEntered();
        installedSessionProductIds.add("workbuddy");
        selectedSessionProductId = "workbuddy";
        renderWorkBuddySnapshot(snapshot);
        syncPanelShape();
      },
      select: setSelectedWorkBuddySession,
      mockHandoff(fail = false) {
        globalThis.handoffCalls = 0;
        globalThis.handoffRequestKeys = [];
        workBuddyHandoffUi.open = async (requestKey) => {
          globalThis.handoffCalls++;
          globalThis.handoffRequestKeys.push(requestKey);
          await new Promise(resolve => globalThis.resolveHandoff = resolve);
          if (fail) throw new Error("未找到可切换的 WorkBuddy 窗口");
        };
      },
      restoreHandoff() { workBuddyHandoffUi.open = openWorkBuddyRequest; },
      get questionId() { return activeQuestionRequest?.id; },
      get view() { return requestedContentView; },
      get displayed() { return displayedContentView; }
    };\n` });
  });
  await page.goto(base);
  try { await page.waitForFunction(() => !!globalThis.codecraftWorkBuddyTest); }
  catch (error) { console.error("Browser initialization errors:", errors); throw error; }
  const now = Date.now();
  const session = {
    id: "wbs_ui_1", workbuddySessionId: "native-ui", workbuddyVersion: "37.10.3-24",
    title: "Review WorkBuddy plan", titleSource: "native",
    cliVersion: "2.137.1", pluginInstanceId: "plugin-ui", processInstanceId: "10:1",
    identityStatus: "observedLineage", source: "CLI", cwdHash: "cwd-safe-hash",
    transcriptHash: null, permissionMode: "default", stage: "waitingForInput",
    currentTool: "ExitPlanMode", lastPrompt: null, eventCount: 1, startedAt: now,
    updatedAt: now, endedAt: null, pendingCount: 0, activities: [], outputs: [],
  };
  const snapshot = {
    connected: true, integrationError: null, version: 1,
    sessions: [session, { ...session, id: "wbs_ui_2", workbuddySessionId: "native-two" }],
    interactions: [], observedEventCount: 2, unknownEventCount: 0,
    capabilities: { canObserve: true, canApproveTools: false, canAnswerQuestions: false, canApprovePlans: false, canStreamOutput: false, protocolFrozen: false, reason: "只读" },
  };
  const empty = { ...snapshot, connected: false, sessions: [], observedEventCount: 0 };
  const pending = { state: "syncedRestartRequired", filesInstalled: true, enabled: true,
    registered: false, loaded: false, connected: false, bridgeReady: true, error: null };
  await page.evaluate(({hook, snapshot}) => globalThis.codecraftWorkBuddyTest.hook(hook, snapshot), {
    hook: {...pending, state: "notInstalled", filesInstalled: false, enabled: false}, snapshot: empty,
  });
  await page.locator('#settings-open').click();
  await page.locator('#settings-tab-hook').click();
  await page.getByRole('button', {name: '安装 WorkBuddy Hook', exact: true}).click();
  const installedHook = page.locator('#installed-hook-list').getByRole('button', {name: '卸载 WorkBuddy Hook', exact: true});
  await installedHook.waitFor({state: 'visible'});
  assert.ok((await installedHook.textContent()).includes('已安装 · 等待加载'));
  await page.locator('#settings-back').click();
  await page.locator('#workbuddy-session-card').waitFor({state: 'visible'});
  assert.equal(await page.locator('#workbuddy-connection-status').getAttribute('data-connected'), 'false');
  assert.equal(await page.locator('#workbuddy-connection-status').getAttribute('title'), '等待 WorkBuddy Hook 加载');
  assert.equal(await page.locator('#workbuddy-session-card > :not(header):not(ul)').count(), 0);
  await page.waitForFunction(() => globalThis.codecraftWorkBuddyTest.displayed === 'sessions');
  await page.evaluate(() => Promise.allSettled(document.getAnimations().filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity).map(animation => animation.finished)));
  await page.screenshot({path: path.join(output, 'hook-waiting-for-load.png')});
  // The pending install remains uninstallable, rather than triggering install again.
  await page.locator('#settings-open').click();
  await installedHook.click();
  await page.locator('#available-hook-list').getByRole('button', {name: '安装 WorkBuddy Hook', exact: true}).waitFor({state: 'visible'});
  assert.equal(await page.locator('#workbuddy-session-card').evaluate(el => el.hidden), true);
  await page.getByRole('button', {name: '安装 WorkBuddy Hook', exact: true}).click();
  await installedHook.waitFor({state: 'visible'});
  await page.locator('#settings-back').click();
  await page.evaluate(({hook, snapshot}) => globalThis.codecraftWorkBuddyTest.hook(hook, snapshot), {
    hook: {...pending, state: 'disabled', enabled: false}, snapshot: empty,
  });
  await page.locator('#workbuddy-session-card').waitFor({state: 'visible'});
  assert.equal(await page.locator('#workbuddy-connection-status').getAttribute('title'), '等待 WorkBuddy Hook 加载');
  assert.equal(await page.locator('#installed-hook-list [aria-label="卸载 WorkBuddy Hook"]').count(), 1);
  await page.evaluate(({hook, snapshot}) => globalThis.codecraftWorkBuddyTest.hook(hook, snapshot), {
    hook: {...pending, loaded: true, connected: false, error: '测试：连接中断'}, snapshot: empty,
  });
  assert.equal(await page.locator('#workbuddy-connection-status').getAttribute('title'), '等待 WorkBuddy Hook 加载');
  await page.evaluate(({hook, snapshot}) => globalThis.codecraftWorkBuddyTest.hook(hook, snapshot), {
    hook: {...pending, loaded: true, connected: true}, snapshot,
  });
  assert.equal(await page.locator('#workbuddy-connection-status').getAttribute('data-connected'), 'true');
  console.log('Hook lifecycle: install moves card, compact session card remains visible while waiting/disabled, pending uninstall, and connection recovery passed.');
  await page.evaluate((snapshot) => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.waitForFunction(() => globalThis.codecraftWorkBuddyTest.displayed === 'sessions');
  const first = page.locator('#workbuddy-session-list button[data-session-id="wbs_ui_1"]');
  await first.waitFor({ state: "visible" });
  assert.equal(await first.isDisabled(), true);
  assert.equal(await first.getAttribute("aria-disabled"), "true");
  assert.equal(await first.locator(".session-button__title").textContent(), "Review WorkBuddy plan");
  assert.equal(await first.locator(".session-button__title").getAttribute("title"), "Review WorkBuddy plan");
  assert.equal(await first.getAttribute("title"), "WorkBuddy 会话详情已禁用 · Review WorkBuddy plan");
  await page.screenshot({path: path.join(output, 'workbuddy-session-titles.png')});
  snapshot.sessions[0].stage = "toolRunning";
  snapshot.sessions[0].currentTool = "Read";
  snapshot.sessions[0].updatedAt++;
  await page.evaluate((snapshot) => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  assert.equal(await first.locator(".sr-only").textContent(), "调用工具中");
  assert.equal(await first.getAttribute("data-session-status"), "working");
  snapshot.sessions.reverse();
  await page.evaluate((snapshot) => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.evaluate(() => globalThis.codecraftWorkBuddyTest.select("wbs_ui_1"));
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.displayed), "sessions");
  assert.equal(await page.locator("#session-detail-view").isVisible(), false);
  snapshot.sessions = [];
  await page.evaluate((snapshot) => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.view), "sessions");
  assert.equal(await page.locator("#session-output").textContent(), "");
  assert.deepEqual(errors, []);
  console.log("Desktop: WorkBuddy sessions remain visible for observation but cannot open the detail page.");

  const question = { requestKey: "wbk_question", sessionId: session.id, kind: "question", status: "pending", answerable: false,
    toolName: "AskUserQuestion", summary: "Choose mode", nativeRequestId: null, toolInput: null, plan: null, payloadHash: "hash", capturedAt: now, expiresAt: now + 300000, reason: "只读观察",
    questions: [{ question: "选择工作模式？", header: "模式", options: [{ label: "只读", description: "观察" }, { label: "完整", description: "执行" }], multiSelect: false, allowOther: true }] };
  const plan = { ...question, requestKey: "wbk_plan", kind: "plan", toolName: "ExitPlanMode", questions: [], plan: "# 当前计划\n1. 读取 input.txt。" };
  const tool = { ...question, requestKey: "wbk_tool", kind: "tool", questions: [], toolName: "Write", summary: "创建 output.txt" };
  snapshot.sessions = [session];

  const assertReadonlyReview = async (interaction, view, openButton) => {
    snapshot.interactions = [interaction];
    await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
    await page.waitForFunction(expected => globalThis.codecraftWorkBuddyTest.view === expected, view);
    await page.locator(openButton).waitFor({state:"visible"});
    assert.equal(await page.locator(`${openButton}:visible`).count(), 1);
    assert.equal(await page.locator(`${openButton} + button:visible`).count(), 0);
    await page.screenshot({path: path.join(output, `workbuddy-${view}-reminder.png`)});
    await page.locator(openButton).click();
    assert.match(await page.locator(`#${view}-submit-status`).textContent(), /运行 WorkBuddy 的设备/);
    assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.view), view);
    snapshot.interactions = [{...interaction, status: 'completed'}];
    await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
    await page.waitForFunction(() => !['question', 'plan', 'permission'].includes(globalThis.codecraftWorkBuddyTest.view));
    await page.locator(openButton).waitFor({state: 'hidden'});
  };

  await assertReadonlyReview(question, "question", "#question-open-workbuddy");
  await assertReadonlyReview(plan, "plan", "#plan-open-workbuddy");
  await assertReadonlyReview(tool, "permission", "#permission-open-workbuddy");

  // Keep all three reminders from one session queued while the user handles each.
  const queuedQuestion = {...question, requestKey: 'queue-question', capturedAt: now + 1};
  const queuedPlan = {...plan, requestKey: 'queue-plan', capturedAt: now + 2};
  const queuedTool = {...tool, requestKey: 'queue-tool', capturedAt: now + 3};
  snapshot.interactions = [queuedQuestion, queuedPlan, queuedTool];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.locator('#question-open-workbuddy').waitFor({state:'visible'});
  assert.equal(await page.locator('#question-submit:visible, #question-reject:visible').count(), 0);
  assert.equal(await page.locator('#question-options button:enabled').count(), 0);
  await page.evaluate(() => globalThis.codecraftWorkBuddyTest.mockHandoff());
  await page.locator('#question-open-workbuddy').click();
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  assert.equal(await page.locator('#question-open-workbuddy').isDisabled(), true);
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.view), 'question');
  assert.deepEqual(await page.evaluate(() => globalThis.handoffRequestKeys), ['queue-question']);
  // A late handoff result must not write into the successor's review.
  queuedQuestion.status = 'completed';
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.locator('#plan-open-workbuddy').waitFor({state:'visible'});
  const planStatus = await page.locator('#plan-submit-status').textContent();
  await page.evaluate(() => globalThis.resolveHandoff());
  assert.equal(await page.locator('#plan-submit-status').textContent(), planStatus);
  assert.equal(await page.locator('#plan-auto:visible, #plan-auto-remember:visible, #plan-custom-input:visible').count(), 0);
  await page.locator('#plan-open-workbuddy').click();
  assert.deepEqual(await page.evaluate(() => globalThis.handoffRequestKeys), ['queue-question', 'queue-plan']);
  await page.evaluate(() => globalThis.resolveHandoff());
  await page.waitForFunction(() => !document.querySelector('#plan-open-workbuddy').disabled);
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.view), 'plan');
  queuedPlan.status = 'denied';
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.locator('#permission-open-workbuddy').waitFor({state:'visible'});
  assert.equal(await page.locator('#permission-allow:visible, #permission-always-allow:visible, #permission-deny:visible').count(), 0);
  await page.locator('#permission-open-workbuddy').click();
  assert.deepEqual(await page.evaluate(() => globalThis.handoffRequestKeys), ['queue-question', 'queue-plan', 'queue-tool']);
  await page.evaluate(() => globalThis.resolveHandoff());
  queuedTool.status = 'failed';
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.waitForFunction(() => !['question', 'plan', 'permission'].includes(globalThis.codecraftWorkBuddyTest.view));
  snapshot.interactions = [{...tool, requestKey:'ordinary-tool', status:'unavailable'}];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  assert.ok(!['question', 'plan', 'permission'].includes(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.view)));
  snapshot.interactions = [{...question, requestKey:'ended-question'}];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.locator('#question-open-workbuddy').waitFor({state:'visible'});
  snapshot.sessions = [{...session, stage:'stopped', endedAt: now + 10}];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.waitForFunction(() => !['question', 'plan', 'permission'].includes(globalThis.codecraftWorkBuddyTest.view));
  snapshot.sessions = [session];
  await page.evaluate(() => globalThis.codecraftWorkBuddyTest.restoreHandoff());
  // Identical wording in a new request must still replace the question identity.
  const repeated = {...question, requestKey:'repeated-question'};
  snapshot.interactions = [repeated];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.locator('#question-open-workbuddy').waitFor({state:'visible'});
  repeated.requestKey = 'repeated-question-new';
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.questionId), 'repeated-question-new');
  snapshot.interactions = [];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  console.log('Reminders: native completion/denial/failure, queue stability, handoff identity, late callbacks, and session end passed.');

  snapshot.interactions = [
    { ...question, status: "completed" },
    { ...plan, status: "completed" },
    { ...tool, status: "completed" },
  ];
  await page.evaluate(snapshot => globalThis.codecraftWorkBuddyTest.show(snapshot), snapshot);
  await page.waitForFunction(() => globalThis.codecraftWorkBuddyTest.displayed === 'sessions');
  await page.evaluate(() => globalThis.codecraftWorkBuddyTest.select("wbs_ui_1"));
  assert.equal(await page.evaluate(() => globalThis.codecraftWorkBuddyTest.displayed), "sessions");
  assert.equal(await page.locator('#session-detail-view').isVisible(), false);
  assert.deepEqual(errors, []);
  console.log("Read-only: WorkBuddy requests remain available through the reminder views; session detail stays disabled.");

  const mobile = await browser.newPage({ viewport: { width: 375, height: 812 } });
  const raw = { workbuddy: { ...snapshot, interactions: [question], sessions: [{ ...session, stage: "waitingForInput", outputs: [{ id: "long", text: "x".repeat(1000) }] }] }, allowApprovals: true };
  await mobile.route("**/api/state", (route) => route.fulfill({ json: raw }));
  await mobile.route("**/api/events", (route) => route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }));
  await mobile.goto(base + "/web/lan/index.html");
  await mobile.locator('#session-list button').first().waitFor({ state: "visible" });
  const mobileSession = mobile.locator('#session-list button').first();
  assert.equal(await mobileSession.isDisabled(), true);
  assert.equal(await mobileSession.getAttribute("aria-disabled"), "true");
  assert.equal(await mobileSession.locator('.session-card__title').textContent(), 'Review WorkBuddy plan');
  assert.equal(await mobile.locator('#detail').isVisible(), false);
  assert.equal(await mobile.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
  await mobile.screenshot({ path: path.join(output, "lan-mobile.png") });
  console.log("LAN: WorkBuddy session cards stay visible but cannot open the detail page.");
} finally {
  await browser.close();
}
