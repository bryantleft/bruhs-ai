#!/usr/bin/env node
// Use the installed Android Studio toolchain without changing shell profiles.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, realpathSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(join(root, 'package.json'));
const env = { ...process.env };
const executable = (name) => (process.platform === 'win32' ? `${name}.exe` : name);
const sdk =
  env.ANDROID_HOME ||
  env.ANDROID_SDK_ROOT ||
  (process.platform === 'darwin'
    ? join(homedir(), 'Library', 'Android', 'sdk')
    : process.platform === 'win32'
      ? join(env.LOCALAPPDATA || join(homedir(), 'AppData', 'Local'), 'Android', 'Sdk')
      : join(homedir(), 'Android', 'Sdk'));

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!existsSync(join(sdk, 'platform-tools', executable('adb')))) {
  fail(
    `Android Platform-Tools were not found in ${sdk}. Install them through Android Studio, or set ANDROID_HOME to the installed SDK.`,
  );
}
if (
  env.ANDROID_HOME &&
  env.ANDROID_SDK_ROOT &&
  (!existsSync(env.ANDROID_SDK_ROOT) ||
    realpathSync(env.ANDROID_HOME) !== realpathSync(env.ANDROID_SDK_ROOT))
) {
  fail('ANDROID_HOME and ANDROID_SDK_ROOT name different SDKs. Set them to the same directory.');
}
env.ANDROID_HOME = sdk;
env.ANDROID_SDK_ROOT = sdk;

if (!env.JAVA_HOME && process.platform === 'darwin') {
  const bundled = '/Applications/Android Studio.app/Contents/jbr/Contents/Home';
  if (existsSync(join(bundled, 'bin', 'java'))) env.JAVA_HOME = bundled;
}
if (env.JAVA_HOME && !existsSync(join(env.JAVA_HOME, 'bin', executable('java')))) {
  fail(
    `JAVA_HOME does not contain a Java executable: ${env.JAVA_HOME}. Select a JDK compatible with the project's Gradle version.`,
  );
}
env.PATH = [
  ...(env.JAVA_HOME ? [join(env.JAVA_HOME, 'bin')] : []),
  join(sdk, 'platform-tools'),
  join(sdk, 'emulator'),
  env.PATH || '',
].join(delimiter);

const java = spawnSync('java', ['-version'], { env, encoding: 'utf8' });
if (java.error || java.status !== 0) {
  fail(
    'Java could not start. Install a compatible JDK or set JAVA_HOME to Android Studio’s bundled jbr/Contents/Home directory.',
  );
}
if (process.argv[2] === '--doctor') {
  console.log(`Android SDK: ${sdk}`);
  console.log(`Java: ${env.JAVA_HOME || 'inherited PATH'}`);
  console.log((java.stderr || java.stdout).trim());
  process.exit(0);
}

const cli = join(dirname(require.resolve('expo/package.json')), 'bin', 'cli');
const child = spawn(process.execPath, [cli, 'run:android', ...process.argv.slice(2)], {
  cwd: root,
  env,
  stdio: 'inherit',
});
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => child.kill(signal));
child.once('error', (error) =>
  fail(`The local Expo Android command could not start: ${error.message}`),
);
child.once('exit', (code) => {
  process.exitCode = code ?? 1;
});
