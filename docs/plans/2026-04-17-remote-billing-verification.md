# Remote Billing Verification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make root `pnpm run check` and `pnpm run lint` verify `crates/remote` on the fork without requiring access to the private billing repository.

**Architecture:** Add a small repo-root Node wrapper that temporarily sanitizes `crates/remote/Cargo.toml` and `crates/remote/Cargo.lock` using the same billing-strip pattern already used in `crates/remote/Dockerfile`, runs the requested Cargo command, then restores the original files in a `finally` path. Keep the change local to verification scripts so normal source layout and upstream gating remain intact.

**Tech Stack:** Node.js ESM scripts, built-in `node:test`, pnpm root scripts, Cargo.

---

### Task 1: Add failing tests for temporary billing sanitization

**Files:**
- Create: `scripts/run-remote-without-billing.test.mjs`

**Steps:**
1. Write a test that creates a temp `crates/remote/Cargo.toml` fixture containing the private billing dependency and `vk-billing = ["dep:billing"]`.
2. Run a helper through a callback and assert the callback sees sanitized contents (`vk-billing = []`, no private dependency, no billing comment).
3. Assert the original manifest is restored after the callback finishes.
4. Add a second test that creates a temp `crates/remote/Cargo.lock`, verifies it is removed during the callback, and restored afterward.
5. Run `node --test scripts/run-remote-without-billing.test.mjs` and confirm it fails before implementation.

### Task 2: Implement the minimal wrapper

**Files:**
- Create: `scripts/run-remote-without-billing.mjs`

**Steps:**
1. Implement a small helper that rewrites manifest text using the exact Dockerfile billing-strip rules.
2. Implement a file wrapper that backs up `crates/remote/Cargo.toml` and optional `crates/remote/Cargo.lock`, applies the sanitization, runs a provided async callback, and restores originals in `finally`.
3. Implement the CLI entrypoint so `node scripts/run-remote-without-billing.mjs -- cargo ...` executes the command, inherits stdio, and exits with the child status.
4. Re-run `node --test scripts/run-remote-without-billing.test.mjs` and confirm it passes.

### Task 3: Wire root verification scripts

**Files:**
- Modify: `package.json`

**Steps:**
1. Change `backend:check` to keep `cargo check --workspace` first, then call the new wrapper for `cargo check --manifest-path crates/remote/Cargo.toml`.
2. Change `backend:lint` to keep the main workspace clippy command first, then call the new wrapper for `cargo clippy --manifest-path crates/remote/Cargo.toml --all-targets -- -D warnings`.
3. Keep `backend:format` unchanged unless verification shows it also needs sanitization.

### Task 4: Verify end-to-end and document outcome

**Files:**
- Modify: `docs/plans/2026-04-17-remote-billing-verification.md` (optional notes only if needed)

**Steps:**
1. Run `node --test scripts/run-remote-without-billing.test.mjs`.
2. Run `pnpm run format`.
3. Run `pnpm run check`.
4. Run `pnpm run lint`.
5. Update EFF-287 with what changed, why this approach was chosen, and whether the root verification ticket can now be closed.
