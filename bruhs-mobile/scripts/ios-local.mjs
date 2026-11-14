#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process';
import {
  createWriteStream,
  mkdirSync,
  readFileSync,
  readdirSync,
  statfsSync,
  statSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { StringDecoder } from 'node:string_decoder';
import { finished } from 'node:stream/promises';
import { getConfig } from 'expo/config';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

class SigningInterrupted extends Error {
  constructor(signal) {
    super(`Local iOS command interrupted by ${signal}.`);
    this.signal = signal;
    this.exitCode = signal === 'SIGINT' ? 130 : 143;
  }
}

/** @param {string} value @returns {string} */
export function redactSigningOutput(value) {
  return value
    .replace(/\bBearer\s+[^\s,;"'}]+/gi, 'Bearer [redacted]')
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, '[redacted token]')
    .replace(
      /((?:password|token|secret|credential)\s*["']?\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;}]+)/gi,
      '$1[redacted]',
    )
    .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, '[Xcode account]');
}

/**
 * Keep provisioning diagnostics while omitting account emails/common credential
 * forms. No passwords, API keys or private-key material are command arguments.
 * @param {string} program @param {string[]} args @param {string} cwd
 * @param {import('node:fs').WriteStream} log @param {AbortSignal} signal
 * @returns {Promise<void>}
 */
async function run(program, args, cwd, log, signal) {
  signal.throwIfAborted();
  const child = spawn(program, args, {
    cwd,
    env: process.env,
    detached: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let forwardingError;
  const interrupt = () => {
    if (!child.pid) return;
    const requested =
      signal.reason instanceof SigningInterrupted ? signal.reason.signal : 'SIGTERM';
    try {
      // Each command owns a new process group, including its build descendants.
      process.kill(-child.pid, requested);
    } catch (error) {
      if (error.code !== 'ESRCH') {
        forwardingError = error;
        child.kill('SIGKILL');
      }
    }
  };
  signal.addEventListener('abort', interrupt, { once: true });
  const decoders = [new StringDecoder('utf8'), new StringDecoder('utf8')];
  const pending = ['', ''];
  const emit = (text) => {
    const safe = redactSigningOutput(text);
    process.stdout.write(safe);
    log.write(safe);
  };
  const streams = [child.stdout, child.stderr];
  streams.forEach((stream, index) => {
    stream.on('data', (chunk) => {
      pending[index] += decoders[index].write(chunk);
      const lines = pending[index].split('\n');
      pending[index] = lines.pop() ?? '';
      for (const line of lines) emit(line + '\n');
    });
    stream.on('end', () => {
      pending[index] += decoders[index].end();
      if (pending[index]) emit(pending[index] + '\n');
      pending[index] = '';
    });
  });
  let code;
  try {
    code = await new Promise((done, fail) => {
      child.once('error', fail);
      child.once('close', done);
    });
  } finally {
    signal.removeEventListener('abort', interrupt);
  }
  if (forwardingError)
    throw new Error('Could not stop the local iOS build process group.', {
      cause: forwardingError,
    });
  signal.throwIfAborted();
  if (code !== 0)
    throw new Error(
      `${program} exited with status ${code}. Review the redacted local signing log.`,
    );
}

async function main() {
  const mode = process.argv[2] ?? 'check';
  if (!['check', 'device', 'archive', 'unsigned-archive'].includes(mode))
    throw new Error('Use ios-local.mjs check, device, archive, or unsigned-archive.');
  const archive = mode === 'archive' || mode === 'unsigned-archive';
  const signed = mode !== 'unsigned-archive';
  if (process.platform !== 'darwin') throw new Error('Local iOS builds require macOS and Xcode.');
  process.env.BRUHS_MOBILE_VARIANT ??= mode === 'device' ? 'development' : 'production';
  if (!['development', 'production'].includes(process.env.BRUHS_MOBILE_VARIANT))
    throw new Error('BRUHS_MOBILE_VARIANT must be development or production.');
  const { exp } = getConfig(root, { skipSDKVersionRequirement: true });
  const team = exp.ios?.appleTeamId;
  const bundle = exp.ios?.bundleIdentifier;
  if (!team || !/^[A-Z0-9]{10}$/.test(team) || !bundle)
    throw new Error('The Expo config must provide a valid iOS team and bundle identifier.');
  let apiKeyPath = process.env.BRUHS_APPLE_API_KEY_PATH;
  let apiKeyId = process.env.BRUHS_APPLE_API_KEY_ID;
  let apiIssuer = process.env.BRUHS_APPLE_API_ISSUER_ID;
  const localMetadata = join(
    homedir(),
    'Library',
    'Application Support',
    'bruhs-mobile',
    'apple-signing.json',
  );
  if (
    signed &&
    !apiKeyPath &&
    !apiKeyId &&
    !apiIssuer &&
    statSync(localMetadata, { throwIfNoEntry: false })
  ) {
    let metadata;
    try {
      metadata = JSON.parse(readFileSync(localMetadata, 'utf8'));
    } catch {
      throw new Error(
        `Could not read local Apple signing metadata. Fix or remove ${localMetadata}, or set all three BRUHS_APPLE_API_* variables.`,
      );
    }
    if (
      !metadata ||
      typeof metadata.keyPath !== 'string' ||
      typeof metadata.keyId !== 'string' ||
      typeof metadata.issuerId !== 'string' ||
      !metadata.keyPath ||
      !metadata.keyId ||
      !metadata.issuerId
    )
      throw new Error(
        'Local Apple signing metadata must contain keyPath, keyId and issuerId strings.',
      );
    apiKeyPath = metadata.keyPath;
    apiKeyId = metadata.keyId;
    apiIssuer = metadata.issuerId;
  }
  const apiAuth = [];
  if (signed && (apiKeyPath || apiKeyId || apiIssuer)) {
    if (!apiKeyPath || !apiKeyId || !apiIssuer)
      throw new Error(
        'Set BRUHS_APPLE_API_KEY_PATH, BRUHS_APPLE_API_KEY_ID and BRUHS_APPLE_API_ISSUER_ID together.',
      );
    if (
      !/^[A-Z0-9]{10}$/.test(apiKeyId) ||
      !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(apiIssuer)
    )
      throw new Error('The App Store Connect key identifier or issuer identifier is malformed.');
    const keyPath = resolve(apiKeyPath);
    if (!statSync(keyPath, { throwIfNoEntry: false })?.isFile())
      throw new Error('BRUHS_APPLE_API_KEY_PATH must name the existing private-key file.');
    apiAuth.push(
      '-authenticationKeyPath',
      keyPath,
      '-authenticationKeyID',
      apiKeyId,
      '-authenticationKeyIssuerID',
      apiIssuer,
    );
  }
  const disk = statfsSync(root, { bigint: true });
  const freeGiB = Number(disk.bavail * disk.bsize) / 1024 ** 3;
  console.log(
    `iOS variant: ${process.env.BRUHS_MOBILE_VARIANT}; bundle: ${bundle}; team: ${team}; free space: ${freeGiB.toFixed(1)} GiB.`,
  );
  const identities = spawnSync('security', ['find-identity', '-v', '-p', 'codesigning'], {
    encoding: 'utf8',
  });
  if (identities.status !== 0)
    throw new Error('Could not inspect local signing identity metadata.');
  console.log(
    `Local identity counts — Apple Development: ${(identities.stdout.match(/"Apple Development:/g) ?? []).length}; Apple Distribution: ${(identities.stdout.match(/"Apple Distribution:/g) ?? []).length}; Developer ID Application: ${(identities.stdout.match(/"Developer ID Application:/g) ?? []).length}.`,
  );
  console.log(
    `Provisioning authentication: ${apiAuth.length ? 'existing App Store Connect API key' : 'saved Xcode account'}.`,
  );
  if (mode === 'check') {
    console.log(
      'Local certificate counts do not establish provisioning access. Device/archive modes use the selected Xcode account or API key with automatic provisioning.',
    );
    return;
  }
  const device = process.env.BRUHS_IOS_DEVICE_ID;
  if (mode === 'device' && (!device || !/^[A-Za-z0-9-]+$/.test(device)))
    throw new Error(
      'Set BRUHS_IOS_DEVICE_ID to a reachable, already paired Xcode device identifier.',
    );
  const logs = join(root, 'build', 'ios-signing');
  mkdirSync(logs, { recursive: true, mode: 0o700 });
  const logPath = join(
    logs,
    `${mode}-${process.env.BRUHS_MOBILE_VARIANT}-${new Date().toISOString().replaceAll(':', '-')}.log`,
  );
  const log = createWriteStream(logPath, { mode: 0o600 });
  const controller = new AbortController();
  const { signal } = controller;
  const onInterrupt = () => controller.abort(new SigningInterrupted('SIGINT'));
  const onTerminate = () => controller.abort(new SigningInterrupted('SIGTERM'));
  const onLogError = (error) =>
    controller.abort(new Error(`Could not write the local signing log: ${error.message}`));
  log.on('error', onLogError);
  process.on('SIGINT', onInterrupt);
  process.on('SIGTERM', onTerminate);
  try {
    await new Promise((done, fail) => {
      log.once('open', done);
      log.once('error', fail);
    });
    console.log(`Redacted signing log: ${logPath}`);
    await run(
      'npx',
      ['--no-install', 'expo', 'prebuild', '--platform', 'ios', '--no-install'],
      root,
      log,
      signal,
    );
    await run('pod', ['install'], join(root, 'ios'), log, signal);
    const workspaces = readdirSync(join(root, 'ios')).filter((name) =>
      name.endsWith('.xcworkspace'),
    );
    if (workspaces.length !== 1)
      throw new Error('Expected one generated iOS workspace; inspect ios/ before building.');
    const workspace = workspaces[0];
    const scheme = basename(workspace, '.xcworkspace');
    const args = [
      '-workspace',
      join('ios', workspace),
      '-scheme',
      scheme,
      '-configuration',
      archive ? 'Release' : 'Debug',
      '-destination',
      archive ? 'generic/platform=iOS' : `platform=iOS,id=${device}`,
      '-derivedDataPath',
      'build/ios-device',
      '-jobs',
      '2',
      'COMPILER_INDEX_STORE_ENABLE=NO',
    ];
    // Signing settings belong to the generated application target. Global
    // DEVELOPMENT_TEAM/CODE_SIGNING_ALLOWED=YES would also override CocoaPods'
    // unsigned library/resource targets and needlessly provision each of them.
    if (signed) args.push('-allowProvisioningUpdates', ...apiAuth);
    else args.push('CODE_SIGNING_ALLOWED=NO');
    const archiveName = `${scheme}${signed ? '' : '-unsigned'}.xcarchive`;
    if (mode === 'device') args.push('-allowProvisioningDeviceRegistration', 'build');
    else args.push('-archivePath', `build/ios-signing/${archiveName}`, 'archive');
    await run('xcodebuild', args, root, log, signal);
    console.log(
      archive
        ? `Local ${signed ? 'signed' : 'unsigned'} archive created at build/ios-signing/${archiveName}. ${signed ? 'No store upload or export was requested.' : 'This compile artifact cannot be installed until it is properly signed.'}`
        : 'Device build completed. Install it only on the selected trusted device using Xcode or devicectl.',
    );
  } finally {
    try {
      const closed = finished(log, { cleanup: true });
      if (!log.destroyed) log.end();
      await closed;
    } finally {
      process.off('SIGINT', onInterrupt);
      process.off('SIGTERM', onTerminate);
      log.off('error', onLogError);
    }
  }
}

if (import.meta.main)
  main().catch((error) => {
    console.error(
      redactSigningOutput(error instanceof Error ? error.message : 'Local iOS setup failed.'),
    );
    process.exitCode = error instanceof SigningInterrupted ? error.exitCode : 1;
  });
