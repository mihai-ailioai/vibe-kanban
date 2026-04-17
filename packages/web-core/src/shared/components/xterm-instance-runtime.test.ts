import { beforeEach, describe, expect, it, vi } from 'vitest';

const originalDocument = globalThis.document;

const adapterState = vi.hoisted(() => {
  const order: string[] = [];
  const terminals: MockTerminal[] = [];
  const fitAddons: MockFitAddon[] = [];

  class MockFitAddon {
    constructor() {
      fitAddons.push(this);
    }

    fit() {
      order.push('fit');
    }
  }

  class MockTerminal {
    cols = 132;
    rows = 43;
    options = {
      theme: undefined as unknown,
      fontFamily: undefined as string | undefined,
    };
    loadAddon = vi.fn((_: unknown) => {
      order.push('loadAddon');
    });
    open = vi.fn((_: HTMLElement) => {
      order.push('open');
    });

    constructor(options?: { theme?: unknown; fontFamily?: string }) {
      order.push('terminal');
      this.options.theme = options?.theme;
      this.options.fontFamily = options?.fontFamily;
      terminals.push(this);
    }
  }

  return {
    order,
    terminals,
    fitAddons,
    ensureTerminalInitMock: vi.fn<() => Promise<void>>(),
    getTerminalThemeMock: vi.fn(),
    MockFitAddon,
    MockTerminal,
  };
});

vi.mock('@/shared/lib/terminalAdapter', () => ({
  ensureTerminalInit: adapterState.ensureTerminalInitMock,
  FitAddon: adapterState.MockFitAddon,
  Terminal: adapterState.MockTerminal,
}));

vi.mock('@/shared/lib/terminalTheme', () => ({
  getTerminalTheme: adapterState.getTerminalThemeMock,
}));

async function importRuntime() {
  vi.resetModules();
  return import('./xterm-instance-runtime');
}

describe('xterm-instance-runtime', () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.stubGlobal('document', (originalDocument ?? {}) as Document);
    adapterState.order.length = 0;
    adapterState.terminals.length = 0;
    adapterState.fitAddons.length = 0;
    adapterState.ensureTerminalInitMock.mockReset();
    adapterState.getTerminalThemeMock.mockReset();
    Reflect.deleteProperty(document, 'fonts');
  });

  it('awaits terminal init before creating and opening the terminal', async () => {
    const theme = { background: '#000000' };
    let resolveInit!: () => void;

    adapterState.getTerminalThemeMock.mockReturnValue(theme);
    adapterState.ensureTerminalInitMock.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          adapterState.order.push('ensureTerminalInit');
          resolveInit = resolve;
        })
    );

    const { createTerminalInstance } = await importRuntime();
    const container = {} as HTMLElement;

    const instancePromise = createTerminalInstance(container);

    expect(adapterState.order).toEqual(['ensureTerminalInit']);
    expect(adapterState.terminals).toHaveLength(0);

    resolveInit();

    const { terminal, fitAddon } = await instancePromise;

    expect(adapterState.order).toEqual([
      'ensureTerminalInit',
      'terminal',
      'loadAddon',
      'open',
      'fit',
    ]);
    expect(terminal).toBe(adapterState.terminals[0]);
    expect(fitAddon).toBe(adapterState.fitAddons[0]);
    expect(adapterState.terminals[0]?.loadAddon).toHaveBeenCalledTimes(1);
    expect(adapterState.terminals[0]?.open).toHaveBeenCalledWith(container);
    expect(adapterState.terminals[0]?.options.theme).toBe(theme);
    expect(adapterState.terminals[0]?.options.fontFamily).toBe(
      '"JetBrains Mono", "Symbols Nerd Font Mono", "Symbols Nerd Font", monospace'
    );
  });

  it('waits for the terminal text font and Nerd Font fallbacks before creating the terminal', async () => {
    const theme = { background: '#111111' };
    const resolveFontLoads: Array<() => void> = [];
    const loadMock = vi.fn(
      (font: string) =>
        new Promise<FontFace[]>((resolve) => {
          adapterState.order.push(`loadFont:${font}`);
          resolveFontLoads.push(() => resolve([]));
        })
    );

    adapterState.getTerminalThemeMock.mockReturnValue(theme);
    adapterState.ensureTerminalInitMock.mockResolvedValue();

    Object.defineProperty(document, 'fonts', {
      configurable: true,
      value: {
        load: loadMock,
      },
    });

    const { createTerminalInstance } = await importRuntime();

    const instancePromise = createTerminalInstance({} as HTMLElement);

    await Promise.resolve();

    expect(loadMock.mock.calls).toEqual([
      ['12px "JetBrains Mono"', 'M'],
      ['12px "Symbols Nerd Font Mono"', '\uf115'],
      ['12px "Symbols Nerd Font"', '\uf115'],
    ]);
    expect(adapterState.order).toEqual([
      'loadFont:12px "JetBrains Mono"',
      'loadFont:12px "Symbols Nerd Font Mono"',
      'loadFont:12px "Symbols Nerd Font"',
    ]);
    expect(adapterState.terminals).toHaveLength(0);

    resolveFontLoads.forEach((resolve) => resolve());

    await instancePromise;

    expect(adapterState.order).toEqual([
      'loadFont:12px "JetBrains Mono"',
      'loadFont:12px "Symbols Nerd Font Mono"',
      'loadFont:12px "Symbols Nerd Font"',
      'terminal',
      'loadAddon',
      'open',
      'fit',
    ]);
  });
});
