import { beforeEach, describe, expect, it, vi } from 'vitest';

const initMock = vi.hoisted(() => vi.fn<() => Promise<void>>());

class MockTerminal {}
class MockFitAddon {}

vi.mock('ghostty-web', () => ({
  init: initMock,
  Terminal: MockTerminal,
  FitAddon: MockFitAddon,
}));

async function importTerminalAdapter() {
  vi.resetModules();
  return import('./terminalAdapter');
}

describe('terminalAdapter', () => {
  beforeEach(() => {
    initMock.mockReset();
  });

  it('re-exports Terminal and FitAddon from ghostty-web', async () => {
    const { Terminal, FitAddon } = await importTerminalAdapter();

    expect(Terminal).toBe(MockTerminal);
    expect(FitAddon).toBe(MockFitAddon);
  });

  it('deduplicates concurrent init calls', async () => {
    let resolveInit!: () => void;
    initMock.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        resolveInit = resolve;
      })
    );

    const { ensureTerminalInit } = await importTerminalAdapter();
    const firstInit = ensureTerminalInit();
    const secondInit = ensureTerminalInit();

    expect(firstInit).toBe(secondInit);
    expect(initMock).toHaveBeenCalledTimes(1);

    resolveInit();

    await expect(firstInit).resolves.toBeUndefined();
  });

  it('skips repeat init after the first successful initialization', async () => {
    initMock.mockResolvedValueOnce(undefined);

    const { ensureTerminalInit } = await importTerminalAdapter();

    await ensureTerminalInit();
    await ensureTerminalInit();

    expect(initMock).toHaveBeenCalledTimes(1);
  });

  it('allows retry after a failed initialization', async () => {
    const expectedError = new Error('ghostty wasm failed');
    initMock
      .mockRejectedValueOnce(expectedError)
      .mockResolvedValueOnce(undefined);

    const { ensureTerminalInit } = await importTerminalAdapter();

    await expect(ensureTerminalInit()).rejects.toThrow(expectedError);
    await expect(ensureTerminalInit()).resolves.toBeUndefined();

    expect(initMock).toHaveBeenCalledTimes(2);
  });
});
