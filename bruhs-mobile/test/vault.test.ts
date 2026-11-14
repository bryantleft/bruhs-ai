import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { test } from 'node:test';
import { Vault, type SecretDriver } from '../src/vault.ts';
import type { NativeCommand } from '../src/domain.ts';
import { command, desktop, tasks } from './fixtures.ts';

class MemorySecrets implements SecretDriver {
  readonly data = new Map<string, string>();
  readonly writes: { key: string; value: string }[] = [];
  fail?: (key: string, value: string) => boolean;
  async get(key: string): Promise<string | null> {
    return this.data.get(key) ?? null;
  }
  async set(key: string, value: string): Promise<void> {
    if (this.fail?.(key, value)) throw new Error('keychain unavailable');
    this.data.set(key, value);
    this.writes.push({ key, value });
  }
  async remove(key: string): Promise<void> {
    this.data.delete(key);
  }
}

test('saved hosts survive a new Vault instance without crossing authority tokens', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  assert.equal((await vault.saveHost(desktop)).ok, true);
  assert.equal((await vault.saveHost(tasks)).ok, true);
  const reopened = new Vault(driver, randomUUID, false);
  assert.deepEqual(await reopened.hosts(), { ok: true, value: [desktop, tasks] });
});

test('a large Unicode command survives secure sharding below iOS value limits', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  const large: NativeCommand = {
    ...command,
    action: { kind: 'prompt', thread_id: desktop.id, text: '🦊繁體'.repeat(3000) },
  };
  assert.equal((await vault.savePending(desktop, large)).ok, true);
  assert.deepEqual(await new Vault(driver, randomUUID, false).pending(desktop), {
    ok: true,
    value: large,
  });
  assert.ok(driver.writes.length > 10);
  assert.ok(
    driver.writes.every((write) => new TextEncoder().encode(write.value).byteLength <= 1024),
  );
});

test('a partial secure write preserves the previous committed exact command', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  await vault.savePending(desktop, command);
  const before = new Map(driver.data);
  driver.fail = (key) => key.endsWith('.2');
  const changed: NativeCommand = {
    ...command,
    action: { kind: 'prompt', thread_id: desktop.id, text: 'new text '.repeat(500) },
  };
  assert.equal((await vault.savePending(desktop, changed)).ok, false);
  assert.deepEqual(await vault.pending(desktop), { ok: true, value: command });
  assert.deepEqual(driver.data, before);
});

test('failed pointer commit does not expose the new command or orphan its shards', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  await vault.savePending(desktop, command);
  const before = new Map(driver.data);
  driver.fail = (key) => key === `bruhs.pending.${desktop.id}`;
  assert.equal((await vault.savePending(desktop, { ...command, sequence: 8 })).ok, false);
  assert.deepEqual(await vault.pending(desktop), { ok: true, value: command });
  assert.deepEqual(driver.data, before);
});

test('corrupted or incomplete pointers fail closed and cannot request unbounded shards', async () => {
  for (const value of [
    'not JSON',
    JSON.stringify({ generation: desktop.id, count: 100000 }),
    JSON.stringify({ generation: '../../other', count: 1 }),
    JSON.stringify({ generation: desktop.id, count: 2 }),
  ]) {
    const driver = new MemorySecrets();
    driver.data.set(`bruhs.pending.${desktop.id}`, value);
    const vault = new Vault(driver, randomUUID, false);
    assert.equal((await vault.pending(desktop)).ok, false);
    assert.equal((await vault.savePending(desktop, null)).ok, true);
    assert.deepEqual(await vault.pending(desktop), { ok: true, value: null });
  }
});

test('record budget rejects oversized commands without altering secure state', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  await vault.savePending(desktop, command);
  const before = new Map(driver.data);
  assert.equal(
    (
      await vault.savePending(desktop, {
        ...command,
        action: { kind: 'prompt', thread_id: desktop.id, text: 'x'.repeat(65536) },
      })
    ).ok,
    false,
  );
  assert.deepEqual(driver.data, before);
});

test('uncertain task operations persist separately and cannot become native commands', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  const marker = { id: command.id, description: 'Stop task + children', at: Date.now() };
  await vault.saveTaskPending(tasks, marker);
  await vault.savePending(desktop, command);
  assert.deepEqual(await new Vault(driver, randomUUID, false).taskPending(tasks), {
    ok: true,
    value: marker,
  });
  assert.equal((await vault.pending(tasks)).ok, false);
  assert.equal((await vault.taskPending(desktop)).ok, false);
  assert.equal((await vault.saveTaskPending(desktop, marker)).ok, false);
  assert.equal((await vault.savePending(tasks, command)).ok, false);
});

test('drafts survive a reconnect/restart and remain isolated to their saved host', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  await vault.saveHost(desktop);
  await vault.saveHost(tasks);
  await vault.saveDrafts(desktop, { thread: 'Unsent desktop draft 🦊' });
  await vault.saveDrafts(tasks, { thread: 'Unsent task draft' });
  const reopened = new Vault(driver, randomUUID, false);
  assert.deepEqual(await reopened.drafts(desktop), {
    ok: true,
    value: { thread: 'Unsent desktop draft 🦊' },
  });
  assert.deepEqual(await reopened.drafts(tasks), {
    ok: true,
    value: { thread: 'Unsent task draft' },
  });
});

test('forgetting a host removes its token, pending command and draft while preserving another host', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  await vault.saveHost(desktop);
  await vault.saveHost(tasks);
  await vault.savePending(desktop, command);
  await vault.saveDrafts(desktop, { thread: 'private unsent draft' });
  assert.equal((await vault.removeHost(desktop.id)).ok, true);
  assert.deepEqual(await vault.hosts(), { ok: true, value: [tasks] });
  assert.ok(
    [...driver.data].every(
      ([key, value]) =>
        !key.includes(desktop.id) &&
        !value.includes(desktop.token) &&
        !value.includes('private unsent draft'),
    ),
  );
});

test('serialized writes cannot resurrect an earlier pending command after acknowledgement', async () => {
  const driver = new MemorySecrets(),
    vault = new Vault(driver, randomUUID, false);
  const first = vault.savePending(desktop, command),
    second = vault.savePending(desktop, null);
  assert.equal((await first).ok, true);
  assert.equal((await second).ok, true);
  assert.deepEqual(await vault.pending(desktop), { ok: true, value: null });
  assert.equal(driver.data.size, 0);
});
