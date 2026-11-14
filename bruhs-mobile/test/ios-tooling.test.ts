import assert from 'node:assert/strict';
import { test } from 'node:test';
import { redactSigningOutput } from '../scripts/ios-local.mjs';

test('signing logs redact credential values and account identifiers while retaining actionable Apple errors', () => {
  const output =
    'Apple account owner@example.com: No profiles for dev.bruhs.mobile were found. password="private-pass" token=opaque-secret Authorization: Bearer private-bearer';
  const safe = redactSigningOutput(output);
  for (const secret of ['owner@example.com', 'private-pass', 'opaque-secret', 'private-bearer'])
    assert.equal(safe.includes(secret), false);
  assert.equal(safe.includes('No profiles for dev.bruhs.mobile were found.'), true);
});

test('signing logs remove JSON secrets and raw JWT-shaped values without hiding team metadata', () => {
  const output =
    '{"credential":"private-value","token":"private-token"} eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.signature TEAM=KW4434WNW9';
  const safe = redactSigningOutput(output);
  assert.equal(safe.includes('private-value'), false);
  assert.equal(safe.includes('private-token'), false);
  assert.equal(safe.includes('eyJhbGciOiJIUzI1NiJ9'), false);
  assert.equal(safe.includes('TEAM=KW4434WNW9'), true);
});
