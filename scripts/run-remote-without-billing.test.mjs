import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  sanitizeRemoteManifestText,
  withRemoteBillingSanitized,
} from './run-remote-without-billing.mjs';

const MANIFEST_FIXTURE = `[features]
default = []
vk-billing = ["dep:billing"]

[dependencies]
# private crate for billing functionality
billing = { git = "ssh://git@github.com/BloopAI/vibe-kanban-private", branch = "main", package = "billing", optional = true }
axum = "0.8"
`;

test('sanitizeRemoteManifestText strips private billing dependency and feature target', () => {
  const sanitized = sanitizeRemoteManifestText(MANIFEST_FIXTURE);

  assert.match(sanitized, /^vk-billing = \[\]$/m);
  assert.doesNotMatch(sanitized, /vibe-kanban-private/);
  assert.doesNotMatch(sanitized, /private crate for billing functionality/);
  assert.match(sanitized, /^axum = "0.8"$/m);
});

test('withRemoteBillingSanitized applies temporary manifest edits and restores them', async () => {
  const repoRoot = await fs.mkdtemp(
    path.join(os.tmpdir(), 'vibe-kanban-remote-billing-'),
  );
  const remoteDir = path.join(repoRoot, 'crates', 'remote');
  await fs.mkdir(remoteDir, { recursive: true });

  const manifestPath = path.join(remoteDir, 'Cargo.toml');
  await fs.writeFile(manifestPath, MANIFEST_FIXTURE);

  let manifestSeenInsideCallback = '';

  await withRemoteBillingSanitized(repoRoot, async () => {
    manifestSeenInsideCallback = await fs.readFile(manifestPath, 'utf8');
  });

  assert.match(manifestSeenInsideCallback, /^vk-billing = \[\]$/m);
  assert.doesNotMatch(manifestSeenInsideCallback, /vibe-kanban-private/);

  const restoredManifest = await fs.readFile(manifestPath, 'utf8');
  assert.equal(restoredManifest, MANIFEST_FIXTURE);
});

test('withRemoteBillingSanitized temporarily removes remote Cargo.lock and restores it', async () => {
  const repoRoot = await fs.mkdtemp(
    path.join(os.tmpdir(), 'vibe-kanban-remote-billing-lock-'),
  );
  const remoteDir = path.join(repoRoot, 'crates', 'remote');
  await fs.mkdir(remoteDir, { recursive: true });

  const manifestPath = path.join(remoteDir, 'Cargo.toml');
  const lockPath = path.join(remoteDir, 'Cargo.lock');
  await fs.writeFile(manifestPath, MANIFEST_FIXTURE);
  await fs.writeFile(lockPath, '[[package]]\nname = "billing"\n');

  let lockExistsInsideCallback = true;

  await withRemoteBillingSanitized(repoRoot, async () => {
    lockExistsInsideCallback = await fs
      .access(lockPath)
      .then(() => true)
      .catch(() => false);
  });

  assert.equal(lockExistsInsideCallback, false);
  assert.equal(await fs.readFile(lockPath, 'utf8'), '[[package]]\nname = "billing"\n');
});
