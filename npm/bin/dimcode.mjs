#!/usr/bin/env node
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import path from 'node:path';

const platform = `${process.platform === 'win32' ? 'windows' : process.platform}-${process.arch}`;
if (!['darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'windows-arm64', 'windows-x64'].includes(platform)) {
  console.error(`dimcode does not support ${platform}.`);
  process.exit(1);
}
let binary;
try {
  const metadata = createRequire(import.meta.url).resolve(`dimcode-${platform}/package.json`);
  binary = path.join(path.dirname(metadata), 'bin', process.platform === 'win32' ? 'dimcode.exe' : 'dimcode');
} catch {
  console.error(`Missing dimcode-${platform}. Reinstall dimcode with npm optional dependencies enabled.`);
  process.exit(1);
}
const { name } = createRequire(import.meta.url)('../package.json');
const env = { ...process.env, DIMCODE_NPM_PACKAGE: name };
if (env.DIMCODE_WEB_ASSETS === undefined) {
  try {
    const web = createRequire(import.meta.url).resolve('dimcode-web/package.json');
    env.DIMCODE_WEB_ASSETS = path.join(path.dirname(web), 'web');
  } catch {
  }
}
if (process.platform !== 'win32' && typeof process.execve === 'function') {
  try {
    process.execve(binary, [binary, ...process.argv.slice(2)], env);
  } catch (error) {
    console.error(`Cannot start dimcode: ${error.message}`);
    process.exit(1);
  }
}
const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit', env });
const onInterrupt = () => {};
const onTerminate = () => child.kill('SIGTERM');
process.on('SIGINT', onInterrupt);
process.on('SIGTERM', onTerminate);
child.on('error', (error) => {
  console.error(`Cannot start dimcode: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  process.removeListener('SIGINT', onInterrupt);
  process.removeListener('SIGTERM', onTerminate);
  process.exitCode = signal ? 1 : code ?? 1;
});
