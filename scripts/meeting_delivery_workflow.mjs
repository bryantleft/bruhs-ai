// Opt-in continuation of the real meeting-suggestion acceptance run.
// Capture speech is scripted; suggestion and coding requests use real models.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { cp, copyFile, mkdir, readFile, stat, symlink, unlink, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import path from 'node:path';
import { taskDeliveryChecks } from '../bruhs-web/tests/task_delivery_checks.mjs';

export async function prepareMeetingDelivery({ root, fixture, artifacts, home, wrapperDir, env, report, codingEffort }) {
  const python = spawnSync('python3', ['-c', 'import sys; print(sys.executable)'], { encoding: 'utf8' }).stdout.trim();
  assert(path.isAbsolute(python), 'Python is required for frozen independent grading');
  const runPython = (source, args, timeout) => {
    const result = spawnSync(python, ['-c', source, ...args], { encoding: 'utf8', timeout });
    assert.equal(result.status, 0, result.stderr || result.error?.message || result.stdout);
    return result.stdout.trim();
  };
  const sha = bytes => createHash('sha256').update(bytes).digest('hex');
  const workflow = report.deliveryWorkflow = {
    scope: 'One connected browser workflow with scripted capture speech, a real meeting suggestion and one real coding attempt. No physical audio, native-client, competitor or published-PR evidence.',
    interventionPolicy: 'Owner supplies the public contract reference and a 45-minute planning estimate before Start. No corrective coding feedback, retries or model changes within the attempt. Delivery remains an unexecuted local proposal.',
    humanContextAdditions: [],
  };
  runPython('import sys,json; from pathlib import Path; from types import SimpleNamespace; sys.path.insert(0,sys.argv[1]); from bench.repository.run import freeze; print(json.dumps(freeze(SimpleNamespace(output=Path(sys.argv[2])),Path(sys.argv[3]))))', [root, artifacts, fixture]);
  const snapshot = path.join(artifacts, 'snapshot');
  for (const file of ['scripts/verify-meeting-suggestions.playwright.mjs', 'scripts/meeting_delivery_workflow.mjs', 'bruhs-web/tests/task_delivery_checks.mjs']) {
    const target = path.join(snapshot, file); await mkdir(path.dirname(target), { recursive: true }); await copyFile(path.join(root, file), target);
  }
  await cp(path.join(root, 'bruhs-web/src'), path.join(snapshot, 'bruhs-web/src'), { recursive: true });
  const frozenFixture = path.join(snapshot, 'bench/tasks/huey-leases');
  for (const name of ['LEASED_DELIVERY_SPEC.md', 'tests/test_leased_delivery.py']) {
    assert((await stat(path.join(frozenFixture, 'repo', name))).isFile(), `The public task input is missing: ${name}`);
  }
  const plan = JSON.parse(await readFile(path.join(frozenFixture, 'campaign.json'), 'utf8'));
  const timeoutMs = plan.timeout_seconds_per_provider_and_grading_step * 1000;
  const effort = codingEffort || plan.effort;
  assert(['low', 'medium', 'high', 'xhigh', 'max'].includes(effort), 'Choose a supported coding effort');
  workflow.coding = { model: `codex/${plan.model}`, effort, nativeCliDefaultEffort: plan.effort, timeoutMs, budgetBasis: plan.budget_basis, config: plan.provider_config, noDaemon: plan.native_cli_no_daemon };
  const native = spawnSync('/usr/bin/which', ['codex'], { encoding: 'utf8' }).stdout.trim();
  assert(path.isAbsolute(native), 'Installed Codex is required');
  workflow.codex = { version: spawnSync(native, ['--version'], { encoding: 'utf8' }).stdout.trim(), sha256: sha(await readFile(native)) };
  const claude = spawnSync('/usr/bin/which', ['claude'], { encoding: 'utf8' }).stdout.trim();
  assert(path.isAbsolute(claude), 'Installed Claude is required for this meeting profile');
  workflow.claude = { version: spawnSync(claude, ['--version'], { encoding: 'utf8' }).stdout.trim(), sha256: sha(await readFile(claude)) };
  const auth = path.join(process.env.CODEX_HOME || path.join(homedir(), '.codex'), 'auth.json');
  assert((await stat(auth)).isFile(), 'Existing Codex sign-in is required');
  env.CODEX_HOME = path.join(home, 'codex'); await mkdir(env.CODEX_HOME, { recursive: true, mode: 0o700 });
  const credentialLink = path.join(env.CODEX_HOME, 'auth.json'); await symlink(auth, credentialLink);
  const calls = path.join(artifacts, 'coding-provider-calls.jsonl');
  runPython('import sys,json; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from bench.live.run import provider_wrapper; provider_wrapper(Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4]),sys.argv[5],json.loads(sys.argv[6]),True)', [snapshot, native, path.join(wrapperDir, 'codex'), calls, plan.effort, JSON.stringify(plan.provider_config)]);
  await writeFile(path.join(wrapperDir, 'gh'), '#!/bin/sh\necho "This acceptance run permits no forge commands" >&2\nexit 78\n', { mode: 0o700 });
  Object.assign(env, {
    XDG_DATA_HOME: path.join(home, 'data'), XDG_CONFIG_HOME: path.join(home, 'config'),
    BRUHS_WORKBENCH_DIR: path.join(artifacts, 'workbench'), BRUHS_DELIVERY_DIR: path.join(home, 'delivery'),
    BRUHS_MAIL_DB: path.join(home, 'mail.sqlite3'), BRUHS_REACTIVE_DB: path.join(home, 'reactive.sqlite3'),
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
  });
  const project = path.join(home, 'project'); await cp(path.join(frozenFixture, 'repo'), project, { recursive: true });
  const git = (...args) => {
    const result = spawnSync('/usr/bin/git', ['-C', project, ...args], { env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  git('init', '-b', 'main'); git('add', '.');
  git('-c', 'user.name=acceptance', '-c', 'user.email=acceptance@localhost', 'commit', '-m', 'Pinned Huey acceptance baseline');
  git('remote', 'add', 'origin', 'https://github.com/coleifer/huey.git');
  workflow.project = project; workflow.baseline = git('rev-parse', 'HEAD');
  workflow.server = path.join(artifacts, 'server-under-test');
  await copyFile(path.join(root, 'target/debug/bruhs-server'), workflow.server, constants.COPYFILE_FICLONE);
  workflow.serverSha256 = sha(await readFile(workflow.server));
  workflow.frozenInputs = JSON.parse(runPython('import sys,json,hashlib; from pathlib import Path; root=Path(sys.argv[1]); print(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob("*")) if p.is_file() and "__pycache__" not in p.parts and p.suffix != ".pyc"}))', [snapshot]));
  return {
    model: workflow.coding.model, server: workflow.server,
    initialize: async context => context.addInitScript(({ model, effort }) => {
      if (!localStorage.getItem('bruhs.model')) localStorage.setItem('bruhs.model', model);
      if (!localStorage.getItem('bruhs.permissionMode')) localStorage.setItem('bruhs.permissionMode', 'plan');
      if (!localStorage.getItem('bruhs.effort')) localStorage.setItem('bruhs.effort', effort);
    }, { model: workflow.coding.model, effort }),
    cleanup: async () => { await unlink(credentialLink); workflow.credentialLinkRemoved = true; },
    run: async ({ page, task, origin, check, waitFor, restart }) => {
      const api = async (method, endpoint, body) => {
        const response = await fetch(origin + endpoint, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
        const result = response.status === 204 ? null : await response.json(); assert(response.ok, `${endpoint}: ${JSON.stringify(result)}`); return result;
      };
      const records = async page => page.evaluate(() => new Promise((resolve, reject) => {
        const open = indexedDB.open('bruhs'); open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const db = open.result, tx = db.transaction(['work_items', 'events'], 'readonly');
          const tasks = tx.objectStore('work_items').getAll(), events = tx.objectStore('events').getAll();
          tx.oncomplete = () => { db.close(); resolve({ tasks: tasks.result, events: events.result }); }; tx.onabort = () => { db.close(); reject(tx.error); };
        };
      }));
      const dismissSidebar = async () => { const close = page.getByRole('button', { name: 'Close sidebar', exact: true }); if (await close.isVisible()) await close.click(); };
      const openTask = async () => {
        await page.goto(origin + '/#/tasks'); await page.getByText(task.title, { exact: true }).first().waitFor();
        if (await page.locator('#task-detail-title').isVisible()) await page.locator('aside').filter({ has: page.locator('#task-detail-title') }).getByTitle('Close', { exact: true }).click();
        await dismissSidebar(); await page.getByText(task.title, { exact: true }).first().click();
        await waitFor(() => page.getByRole('button', { name: 'Review agent work', exact: true }).isEnabled(), 'task saves settle');
      };
      await openTask();
      const description = [task.description, 'Implement the contract in LEASED_DELIVERY_SPEC.md and run the public smoke checks in tests/test_leased_delivery.py. Work directly without delegation. Do not publish, merge, contact anyone, access credentials or read other evaluation workspaces.'].filter(Boolean).join('\n\n');
      await page.getByPlaceholder('Add description', { exact: true }).fill(description);
      await page.getByRole('button', { name: /Estimate/ }).click(); await page.getByRole('option', { name: '45 minutes', exact: true }).click();
      await waitFor(async () => { const row = (await records(page)).tasks.find(row => row.id === task.id); return row.description === description && row.planning.estimate_minutes === 45; }, 'owner context and estimate save');
      workflow.humanContextAdditions.push({ kind: 'public-contract-reference', description }, { kind: 'planning-estimate', minutes: 45 });
      await page.goto(origin + '/#/today'); await page.getByRole('heading', { name: 'Today', exact: true }).waitFor(); await dismissSidebar();
      await page.getByRole('button', { name: 'Plan my day', exact: true }).click();
      await page.getByRole('dialog', { name: 'Review day plan' }).getByRole('button', { name: 'Apply plan', exact: true }).click();
      await page.getByRole('button', { name: 'Undo', exact: true }).waitFor();
      const before = await records(page), planned = before.tasks.find(row => row.id === task.id);
      assert.equal(planned.planning.time_block_event_ids.length, 1);
      assert(before.events.some(event => planned.planning.time_block_event_ids.includes(event.id)));
      check('meeting-task-receives-a-real-reviewed-calendar-block', { itemId: task.id, eventIds: planned.planning.time_block_event_ids });
      await openTask();
      const [response] = await Promise.all([page.waitForResponse(response => new URL(response.url()).pathname === '/api/coordination/handoffs' && response.request().method() === 'POST'), page.getByRole('button', { name: 'Review agent work', exact: true }).click()]);
      assert(response.ok()); const handoff = await response.json(); workflow.handoff = handoff;
      assert.deepEqual(handoff.request.item, planned);
      assert.equal(handoff.request.effort, effort);
      const frame = () => page.frameLocator('iframe[title="Local agent workbench"]').frameLocator('#panel');
      await frame().getByRole('heading', { name: 'Review agent work', exact: true }).waitFor();
      await frame().locator('#project').fill(project); await frame().locator('#model').fill(workflow.coding.model);
      assert.equal(await frame().locator('#effort').inputValue(), effort);
      await frame().locator('#access').selectOption('acceptEdits'); await frame().locator('#isolation').selectOption('worktree');
      workflow.startedAt = new Date().toISOString(); await frame().getByRole('button', { name: 'Start task', exact: true }).click();
      let completed;
      await waitFor(async () => {
        const saved = await api('GET', `/api/coordination/handoffs/${handoff.id}`);
        if (!saved.handoff.started) return false;
        completed = (await api('GET', '/api/coordination')).tasks.find(row => row.id === saved.handoff.started.task_id);
        if (completed?.state === 'failed') throw new Error(`The coding attempt failed: ${JSON.stringify(completed)}`);
        return completed?.state === 'completed';
      }, 'one real coding attempt completes', timeoutMs);
      workflow.completedAt = new Date().toISOString(); workflow.run = completed;
      assert.equal(completed.effort, effort);
      assert.equal(completed.model, workflow.coding.model); assert.equal(completed.permission_mode, 'acceptEdits'); assert.notEqual(completed.working_path, project);
      assert.deepEqual(completed.work_item_origin.time_block_event_ids, planned.planning.time_block_event_ids);
      const gradingDir = path.join(artifacts, 'independent-grading');
      const grade = spawnSync(python, [path.join(snapshot, 'bench/repository/grade_cli.py'), '--fixture', frozenFixture, '--work', completed.working_path, '--output', gradingDir, '--timeout', String(timeoutMs / 1000)], { encoding: 'utf8', timeout: timeoutMs + 1000 });
      workflow.grader = { status: grade.status, error: grade.error?.message, stdout: grade.stdout, stderr: grade.stderr, report: JSON.parse(await readFile(path.join(gradingDir, 'report.json'), 'utf8')) };
      workflow.runtime = JSON.parse(runPython('import sys,json; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from bench.repository.run import audit_runtime; print(json.dumps(audit_runtime(Path(sys.argv[2]),Path(sys.argv[3]))))', [snapshot, env.CODEX_HOME, completed.working_path]));
      workflow.runtimeEfforts = JSON.parse(runPython('import sys,json; from pathlib import Path; print(json.dumps([json.loads(line)["payload"].get("effort") for file in (Path(sys.argv[1])/"sessions").rglob("*.jsonl") for line in file.read_text().splitlines() if json.loads(line).get("type")=="turn_context"]))', [env.CODEX_HOME]));
      workflow.providerCalls = (await readFile(calls, 'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse);
      workflow.usage = await api('GET', '/api/agent-extensions/usage');
      assert.equal(workflow.providerCalls.length, 1); assert(workflow.runtime.passed, 'Runtime context audit failed');
      assert(workflow.runtimeEfforts.length && workflow.runtimeEfforts.every(value => value === effort), 'Native runtime effort differs from the reviewed task');
      assert.equal(grade.status, 0, 'Independent frozen feature/regression grading failed');
      check('meeting-task-code-passes-frozen-repository-checks', { expectedTests: workflow.grader.report.expected_tests, runId: completed.id });
      assert.deepEqual(await records(page), before);
      await openTask(); await page.getByRole('button', { name: 'Review agent work', exact: true }).click();
      await frame().getByText('From personal task: ' + task.title, { exact: true }).waitFor();
      const deliveryCheck = (passed, name, actual) => { check(name, { passed, actual }); assert(passed, name); };
      const wait = async (test, label) => { let value; await waitFor(async () => { value = await test(); return Boolean(value); }, label); return value; };
      workflow.delivery = await taskDeliveryChecks({ page, frame, task: completed, before, records, api, check: deliveryCheck, wait, restart,
        reopenRun: async () => { await page.reload(); await openTask(); await page.getByRole('button', { name: 'Review agent work', exact: true }).click(); await frame().getByText('From personal task: ' + task.title, { exact: true }).waitFor(); },
        output: artifacts, name: 'connected-live', env,
      });
      assert.equal(git('rev-parse', 'HEAD'), workflow.baseline);
      assert.equal(git('status', '--porcelain'), '');
      workflow.frozenInputsUnchanged = JSON.parse(runPython('import sys,json,hashlib; from pathlib import Path; root=Path(sys.argv[1]); print(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob("*")) if p.is_file() and "__pycache__" not in p.parts and p.suffix != ".pyc"}))', [snapshot]));
      assert.deepEqual(workflow.frozenInputsUnchanged, workflow.frozenInputs);
      check('connected-workflow-preserves-the-source-repository-and-frozen-grader');
      await page.goto(origin + '/#/tasks'); await page.getByText(task.title, { exact: true }).first().waitFor();
    },
  };
}
