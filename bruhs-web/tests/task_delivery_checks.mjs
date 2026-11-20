// Shared real-host checks for the task-to-delivery review. No forge execution.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

export async function taskDeliveryChecks({ page, frame, task, before, records, api, check, wait, restart, reopenRun, output, name, env }) {
  const prefix = label => `${name}: ${label}`;
  const git = (...args) => {
    const result = spawnSync('/usr/bin/git', ['-C', task.working_path, ...args], { env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout;
  };
  const sha = bytes => createHash('sha256').update(bytes).digest('hex');
  const headBefore = git('rev-parse', 'HEAD').trim();
  const indexFile = path.resolve(task.working_path, git('rev-parse', '--git-path', 'index').trim());
  const indexBefore = sha(await readFile(indexFile));
  const status = git('status', '--porcelain=v1', '-z', '--untracked-files=all').split('\0').filter(Boolean);
  const files = [];
  for (let index = 0; index < status.length; index++) {
    const entry = status[index]; files.push(entry.slice(3));
    if (/[RC]/.test(entry.slice(0, 2))) files.push(status[++index]);
  }
  assert(files.length && files.every(file => file && !file.includes('\n')), 'Fixture changes must fit the selected-file review');
  const openDelivery = async () => {
    const button = frame().getByRole('button', { name: 'Review delivery', exact: true });
    await wait(() => button.isEnabled(), 'settled task enables delivery review');
    await button.scrollIntoViewIfNeeded(); await button.focus(); await page.keyboard.press('Enter');
    await frame().getByRole('heading', { name: 'Code delivery', exact: true }).waitFor();
    await wait(async () => await frame().locator('#repo').inputValue() === task.working_path, 'host resolves the source worktree');
  };
  await openDelivery();
  check(await frame().locator('#repo').evaluate(element => element.readOnly) && await frame().locator('#head').inputValue() === git('branch', '--show-current').trim(), prefix('delivery receives the actual task worktree and branch'));
  const origin = task.work_item_origin;
  await frame().locator('#task-source').getByText('Personal task: ' + origin.item_title, { exact: true }).waitFor();
  await frame().locator('#task-source').getByText('Source meeting: ' + origin.source.title, { exact: true }).waitFor();
  await frame().locator('#action').selectOption('commit_push_pull');
  await frame().locator('#title').fill('Owner reviewed implementation');
  await frame().locator('#body').fill('Owner reviewed public description.');
  await frame().locator('#files').fill([...new Set(files)].join('\n'));
  await frame().locator('#commit-message').fill('fix: implement reviewed task');
  // The owner explicitly supplies the same observation budget as this test.
  await frame().locator('#timeout').fill('60');
  const [response] = await Promise.all([
    page.waitForResponse(response => new URL(response.url()).pathname === `/api/delivery/tasks/${task.id}/prepare`),
    frame().getByRole('button', { name: 'Preview action', exact: true }).click(),
  ]);
  assert(response.ok(), await response.text());
  const prepared = await response.json();
  check(prepared.path === task.working_path && prepared.source_task.task_id === task.id && JSON.stringify(prepared.source_task.work_item_origin) === JSON.stringify(origin), prefix('proposal freezes the typed run, personal task, meeting and calendar source'), prepared.source_task);
  check(prepared.action.body === 'Owner reviewed public description.' && !JSON.stringify(prepared.commands).includes(origin.item_id), prefix('source details do not enter the public description or commands'));
  const saved = (await api('GET', '/api/delivery')).actions.find(receipt => receipt.prepared.id === prepared.id);
  check(saved.status === 'prepared' && saved.completed_commands === 0 && git('rev-parse', 'HEAD').trim() === headBefore && sha(await readFile(indexFile)) === indexBefore, prefix('preview runs no delivery command and leaves Git head and index intact'));
  const request = response.request().postDataJSON();
  assert(!Object.hasOwn(request, 'path') && !Object.hasOwn(request, 'source_task'));
  const beforeForged = (await api('GET', '/api/delivery')).actions.map(receipt => receipt.prepared.id);
  const forged = await page.request.post(new URL(`/api/delivery/tasks/${task.id}/prepare`, page.url()).href, { data: { ...request, path: task.project_path } });
  check(!forged.ok() && JSON.stringify((await api('GET', '/api/delivery')).actions.map(receipt => receipt.prepared.id)) === JSON.stringify(beforeForged), prefix('task preparation rejects a substituted repository path'), { status: forged.status() });

  await frame().locator('#proposal').getByRole('button', { name: 'Open source run', exact: true }).click();
  await frame().getByText('From personal task: ' + origin.item_title, { exact: true }).waitFor();
  check(await frame().getByRole('heading', { name: task.title, exact: true }).isVisible(), prefix('delivery returns to the exact source run'));
  await restart(); await reopenRun(); await openDelivery();
  const receiptRow = frame().locator(`#receipts details[data-id="${prepared.id}"]`);
  await receiptRow.locator('summary').first().click();
  await receiptRow.getByRole('button', { name: 'Review action', exact: true }).click();
  await frame().locator('#proposal').getByText('Source meeting: ' + origin.source.title, { exact: true }).waitFor();
  const restored = (await api('GET', '/api/delivery')).actions.find(receipt => receipt.prepared.id === prepared.id);
  check(JSON.stringify(restored.prepared) === JSON.stringify(prepared) && restored.status === 'prepared', prefix('host restart restores the exact unexecuted proposal and source'));
  await page.screenshot({ path: path.join(output, `${name}-delivery-review.png`) });
  check(await frame().locator('html').evaluate(element => element.scrollWidth <= innerWidth), prefix('delivery source and review fit the viewport'));
  await frame().locator('#proposal').getByRole('button', { name: 'Remove source details', exact: true }).click();
  await wait(async () => !(await api('GET', '/api/delivery')).actions.find(receipt => receipt.prepared.id === prepared.id).prepared.source_task, 'receipt source removal persists');
  await restart();
  const forgotten = (await api('GET', '/api/delivery')).actions.find(receipt => receipt.prepared.id === prepared.id);
  check(!forgotten.prepared.source_task && JSON.stringify(forgotten.prepared.commands) === JSON.stringify(prepared.commands) && forgotten.status === 'prepared' && JSON.stringify(await records(page)) === JSON.stringify(before), prefix('source removal survives restart and preserves reviewed commands and personal data'));
  const unavailable = new URL('/api/delivery/ui', page.url());
  unavailable.searchParams.set('task', 'missing-source-task'); unavailable.searchParams.set('path', task.project_path);
  const beforeMissing = (await api('GET', '/api/delivery')).actions.map(receipt => receipt.prepared.id);
  await page.goto(unavailable.href); await page.getByRole('alert').waitFor();
  assert.equal(await page.locator('#repo').inputValue(), '');
  await page.locator('#action-details > summary').click();
  await page.locator('#title').fill('Should not prepare'); await page.locator('#head').fill(prepared.expected_branch); await page.locator('#timeout').fill('60');
  await page.getByRole('button', { name: 'Preview action', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'Load the source run' }).waitFor();
  check(JSON.stringify((await api('GET', '/api/delivery')).actions.map(receipt => receipt.prepared.id)) === JSON.stringify(beforeMissing), prefix('missing source cannot fall back to a different repository from the URL'));
  return { prepared, files, indexBefore, headBefore };
}
