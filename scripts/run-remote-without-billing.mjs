import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

export function sanitizeRemoteManifestText(text) {
  return text
    .replace(/^billing = \{.*vibe-kanban-private.*\}\n?/m, '')
    .replace(/^# private crate for billing functionality\n?/m, '')
    .replace(/^vk-billing = \["dep:billing"\]$/m, 'vk-billing = []');
}

export async function withRemoteBillingSanitized(repoRoot, callback) {
  const remoteDir = path.join(repoRoot, 'crates', 'remote');
  const manifestPath = path.join(remoteDir, 'Cargo.toml');
  const lockPath = path.join(remoteDir, 'Cargo.lock');

  const originalManifest = await fs.readFile(manifestPath, 'utf8');
  const sanitizedManifest = sanitizeRemoteManifestText(originalManifest);
  const originalLock = await fs.readFile(lockPath).catch((error) => {
    if (error.code === 'ENOENT') {
      return null;
    }
    throw error;
  });

  await fs.writeFile(manifestPath, sanitizedManifest);
  if (originalLock !== null) {
    await fs.rm(lockPath, { force: true });
  }

  try {
    return await callback();
  } finally {
    await fs.writeFile(manifestPath, originalManifest);
    if (originalLock !== null) {
      await fs.writeFile(lockPath, originalLock);
    }
  }
}

function runCommand(command, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command[0], command.slice(1), {
      cwd,
      stdio: 'inherit',
    });

    child.on('error', reject);
    child.on('close', (code, signal) => {
      if (signal) {
        resolve(1);
        return;
      }

      resolve(code ?? 1);
    });
  });
}

async function main() {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const command = process.argv.slice(2).filter((arg, index) => {
    return !(index === 0 && arg === '--');
  });

  if (command.length === 0) {
    throw new Error(
      'Usage: node scripts/run-remote-without-billing.mjs -- <command> [args...]',
    );
  }

  const exitCode = await withRemoteBillingSanitized(repoRoot, () =>
    runCommand(command, repoRoot),
  );

  process.exit(exitCode);
}

const entrypointPath = process.argv[1]
  ? path.resolve(process.argv[1])
  : null;
const modulePath = fileURLToPath(import.meta.url);

if (entrypointPath === modulePath) {
  await main();
}
