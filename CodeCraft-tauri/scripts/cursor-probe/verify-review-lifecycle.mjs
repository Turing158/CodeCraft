// Synthetic desktop/LAN lifecycle checks. No real approvals are submitted.
// Run after dot-sourcing limit-cpu.ps1 so Vite and Chromium inherit two cores.
import assert from 'node:assert/strict';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createServer } from 'vite';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const root = fileURLToPath(new URL('../../', import.meta.url));
const server = await createServer({ root, server: { host: '127.0.0.1', port: 1422, strictPort: true } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE });
  const page = await browser.newPage({ viewport: { width: 760, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: await response.text() + `
window.reviewFixture = async (kind, options = {}) => {
  await startupSequencePromise;
  controller.setCollapseDelay(60000);
  await controller.pointerEntered();
  closeCursorRequest();
  collapseExpandSettings.approvalAutoExpand = false;
  switchContentView('sessions');
  const { createCursorInteractionPreview } = await import('/src/cursor-preview.ts');
  const s = createCursorInteractionPreview();
  s.appEpoch = 'lifecycle-' + (window.fixtureNumber = (window.fixtureNumber || 0) + 1);
  s.requests.forEach(r => r.submission.appEpoch = s.appEpoch);
  const r = s.requests.find(r => r.body.kind === kind);
  const queued = s.requests.find(r => r.body.kind !== kind);
  s.requests = options.queue ? [r, queued] : [r];
  s.sessions = s.sessions.filter(session => s.requests.some(r => r.sessionKey === session.key));
  window.reviewRaw = s;
  window.reviewCalls = [];
  window.reviewRefreshStarted = false;
  cursorView.submit = async (route, body) => {
    window.reviewCalls.push({route,body});
    if (options.failure) throw Error('synthetic submission failed');
    if (options.deferred) await new Promise((resolve,reject) => {
      window.finishSubmission = resolve; window.failSubmission = () => reject(Error('late old error'));
    });
  };
  // A stuck read must never hold an already acknowledged form open.
  cursorView.refresh = async () => { window.reviewRefreshStarted = true; await new Promise(() => {}); };
  renderCursorSnapshot(s);
  if (options.detail) openCursorSession(r.sessionKey);
  openCursorRequest(r.requestId);
  collapseExpandSettings.approvalAutoExpand = true;
  syncPanelShape();
};
window.reviewState = () => ({ view: requestedContentView, active: activeCursorRequestId,
  calls: window.reviewCalls.length, refresh: window.reviewRefreshStarted,
  waiting: latestCursorSnapshot.sessions.filter(s => s.hasCurrentRequest).length,
  pending: latestCursorSnapshot.requests.filter(r => r.actionable).map(r => r.requestId) });
window.reviewPoll = change => {
  const s = window.reviewRaw, r = s.requests[0];
  s.version++;
  if (change === 'remove') { s.requests = []; s.sessions = []; }
  else if (change === 'offline') s.connected = false;
  else if (change === 'online') s.connected = true;
  else if (change === 'revise') s.requests[0] = { ...r,
    submission: { ...r.submission, requestVersion: r.submission.requestVersion + 1 },
    body: { ...r.body, content: { ...r.body.content, toolInput: { command: 'echo revised' } } } };
  else if (change !== 'pending') { r.phase = change; r.actionable = false; s.sessions[0].hasCurrentRequest = false; }
  renderCursorSnapshot(s);
};
window.reviewSettings = () => switchContentView('settings');
window.reviewBack = closeCursorRequest;
` });
  });
  await page.goto('http://127.0.0.1:1422/?cursor=1');
  await page.waitForFunction(() => window.reviewFixture);
  const form = page.locator('.cursor-review-form');
  const fixture = async (kind, options = {}) => {
    await page.evaluate(([kind, options]) => window.reviewFixture(kind, options), [kind, options]);
    await form.waitFor({ state: 'visible' });
  };
  const viewIs = view => page.waitForFunction(view => window.reviewState().view === view, view);
  const submit = async kind => {
    if (kind === 'question') {
      await form.getByRole('button', { name: '开发环境', exact: true }).click();
      await form.getByRole('button', { name: '下一步', exact: true }).click();
      await form.getByRole('button', { name: '跳过此题', exact: true }).click();
      await form.getByRole('button', { name: '提交', exact: true }).click();
    } else if (kind === 'plan') {
      await form.getByRole('button', { name: '批准计划', exact: false }).click();
    } else await form.getByRole('button', { name: '允许一次', exact: false }).click();
  };

  for (const kind of ['question', 'plan', 'permission']) {
    await fixture(kind);
    await submit(kind);
    await viewIs('sessions');
    const state = await page.evaluate(() => window.reviewState());
    assert.equal(state.calls, 1);
    assert.equal(state.waiting, 0);
    assert.equal(state.refresh, true);
    assert.deepEqual(state.pending, []);
    await page.evaluate(() => window.reviewPoll('pending'));
    assert.equal((await page.evaluate(() => window.reviewState())).view, 'sessions');
    assert.equal(await form.count(), 0);
  }
  console.log('Passed: all three submissions close before a stalled refresh and ignore older pending polls.');

  for (const kind of ['question', 'plan', 'permission']) {
    for (const phase of ['decided', 'expired', 'cancelled', 'superseded', 'disconnected']) {
      await fixture(kind, { detail: true });
      await page.evaluate(phase => window.reviewPoll(phase), phase);
      await viewIs('detail');
      assert.equal(await form.count(), 0);
    }
  }
  await fixture('question');
  await page.evaluate(() => window.reviewPoll('remove'));
  await viewIs('sessions');
  await fixture('permission', { queue: true });
  await submit('permission');
  await viewIs('question');
  assert.equal((await page.evaluate(() => window.reviewState())).active, 'synthetic-question');
  console.log('Passed: remote decisions and terminal requests close; queued requests open immediately.');

  await fixture('permission', { failure: true });
  await submit('permission');
  await page.waitForFunction(() => document.querySelector('.cursor-review-form [role=alert]')?.textContent.includes('synthetic submission failed'));
  assert.equal((await page.evaluate(() => window.reviewState())).view, 'permission');
  assert.equal(await form.getByRole('button', { name: '允许一次', exact: false }).isEnabled(), true);
  await fixture('permission');
  await page.evaluate(() => window.reviewPoll('offline'));
  assert.equal(await form.getByRole('button', { name: '允许一次', exact: false }).isEnabled(), false);
  await page.evaluate(() => window.reviewPoll('online'));
  assert.equal(await form.getByRole('button', { name: '允许一次', exact: false }).isEnabled(), true);
  for (const finish of ['finishSubmission', 'failSubmission']) {
    await fixture('permission', { deferred: true });
    await submit('permission');
    await page.evaluate(() => window.reviewPoll('revise'));
    assert.equal(await form.getByRole('button', { name: '允许一次', exact: false }).isEnabled(), true);
    await page.evaluate(finish => window[finish](), finish);
    assert.equal((await page.evaluate(() => window.reviewState())).view, 'permission');
    assert.equal(await form.locator('[role=alert]').textContent(), '');
  }
  await fixture('question', { detail: true });
  await page.evaluate(() => { window.reviewBack(); window.reviewPoll('pending'); });
  await viewIs('detail');
  await fixture('plan');
  await page.evaluate(() => { window.reviewSettings(); window.reviewPoll('decided'); });
  await viewIs('settings');
  console.log('Passed: failure/offline behavior, revised-form races, manual back and settings navigation.');

  const lan = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  lan.on('pageerror', error => errors.push(error.message));
  await lan.route('**/web/lan/main.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: await response.text() + `
window.lanReviewFixture = async kind => {
  const { createCursorInteractionPreview } = await import('/src/cursor-preview.ts');
  const s = createCursorInteractionPreview();
  s.appEpoch = 'lan-lifecycle-' + kind;
  s.requests = s.requests.filter(r => r.body.kind === kind);
  s.requests[0].submission.appEpoch = s.appEpoch;
  s.sessions = s.sessions.filter(session => session.key === s.requests[0].sessionKey);
  cursorView.submit = async () => {};
  cursorView.refresh = async () => { await new Promise(() => {}); };
  selectedSources.clear();
  applySnapshot({ generatedAt: Date.now(), allowApprovals: true, cursor: s });
  selectEntry('cursor:' + s.sessions[0].key, 'detail');
};
` });
  });
  await lan.goto('http://127.0.0.1:1422/web/lan/index.html?cursor=1');
  await lan.waitForFunction(() => window.lanReviewFixture);
  for (const [kind, label] of [['permission', '允许本次'], ['question', '取消问题'], ['plan', '批准计划']]) {
    await lan.evaluate(kind => window.lanReviewFixture(kind), kind);
    await lan.locator('.cursor-interactions').getByRole('button', { name: label, exact: true }).click();
    await lan.locator('.cursor-interactions').waitFor({ state: 'detached' });
  }
  console.log('Passed: LAN removes all three acknowledged forms before a stalled refresh.');
  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  await server.close();
}
