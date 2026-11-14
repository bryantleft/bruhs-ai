import assert from 'node:assert/strict';
import { test } from 'node:test';
import { RecordingGate } from '../src/recordingGate.ts';

test('microphone consent resolving after backgrounding cannot start recording', async () => {
  const gate = new RecordingGate();
  gate.mount(true);
  const attempt = gate.begin();
  assert.notEqual(attempt, null);
  let resolvePermission: (() => void) | undefined;
  const permission = new Promise<void>((resolve) => {
    resolvePermission = resolve;
  });
  let started = false;
  const start = permission.then(() => {
    if (attempt !== null && gate.current(attempt)) started = true;
  });
  gate.setForeground(false);
  resolvePermission?.();
  await start;
  assert.equal(started, false);
  gate.setForeground(true);
  assert.equal(attempt !== null && gate.current(attempt), false);
  assert.notEqual(gate.begin(), null);
});

test('offline-model lookup resolving after cancellation or unmount remains cancelled', async () => {
  for (const cancel of [
    (gate: RecordingGate) => gate.cancel(),
    (gate: RecordingGate) => gate.unmount(),
  ]) {
    const gate = new RecordingGate();
    gate.mount(true);
    const attempt = gate.begin();
    assert.notEqual(attempt, null);
    await Promise.resolve();
    cancel(gate);
    await Promise.resolve();
    assert.equal(attempt !== null && gate.current(attempt), false);
  }
});

test('a superseded permission callback cannot finish or unlock a newer attempt', () => {
  const gate = new RecordingGate();
  gate.mount(true);
  const old = gate.begin();
  assert.notEqual(old, null);
  assert.equal(gate.begin(), null);
  gate.cancel();
  const current = gate.begin();
  assert.notEqual(current, null);
  assert.equal(old !== null && gate.finish(old), false);
  assert.equal(gate.begin(), null);
  assert.equal(current !== null && gate.finish(current), true);
  assert.notEqual(gate.begin(), null);
});

test('remount restores availability while invalidating work from the old mount', () => {
  const gate = new RecordingGate();
  gate.mount(true);
  const old = gate.begin();
  gate.unmount();
  assert.equal(gate.begin(), null);
  gate.mount(true);
  assert.equal(old !== null && gate.current(old), false);
  const current = gate.begin();
  assert.notEqual(current, null);
  assert.equal(current !== null && gate.current(current), true);
});
