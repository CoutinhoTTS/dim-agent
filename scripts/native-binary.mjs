import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

export const platforms = ['darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'windows-arm64', 'windows-x64'];

/** The platform being built: DIM_BUILD_PLATFORM for a cross build, else the host. */
export function buildPlatform() {
  const host = `${process.platform === 'win32' ? 'windows' : process.platform}-${process.arch}`;
  const platform = process.env.DIM_BUILD_PLATFORM || host;
  assert(platforms.includes(platform), 'Unsupported native platform');
  return { platform, host, osName: platform.split('-')[0], cpu: platform.split('-')[1] };
}

/** Rejects an executable whose format or CPU does not match the platform. */
export function checkArchitecture(file, osName, cpu) {
  const data = readFileSync(file);
  if (osName === 'windows') {
    assert.equal(data.readUInt16LE(0), 0x5a4d, 'Expected a PE executable');
    const offset = data.readUInt32LE(0x3c);
    assert.equal(data.readUInt32LE(offset), 0x00004550, 'Invalid PE signature');
    assert.equal(data.readUInt16LE(offset + 4), cpu === 'arm64' ? 0xaa64 : 0x8664, 'PE architecture mismatch');
  } else if (osName === 'darwin') {
    assert.equal(data.readUInt32LE(0), 0xfeedfacf, 'Expected a 64-bit Mach-O executable');
    assert.equal(data.readUInt32LE(4), cpu === 'arm64' ? 0x0100000c : 0x01000007, 'Mach-O architecture mismatch');
  } else {
    assert.equal(data.subarray(0, 6).toString('hex'), '7f454c460201', 'Expected a little-endian ELF64 executable');
    assert.equal(data.readUInt16LE(18), cpu === 'arm64' ? 183 : 62, 'ELF architecture mismatch');
  }
}
