import {
  FitAddon,
  Terminal,
  ensureTerminalInit,
} from '@/shared/lib/terminalAdapter';
import { getTerminalTheme } from '@/shared/lib/terminalTheme';

const TERMINAL_FONT_SIZE = 12;
const TERMINAL_ICON_SAMPLE = '\uf115';
const TERMINAL_FONT_LOADS = [
  { family: 'JetBrains Mono', sample: 'M' },
  { family: 'Symbols Nerd Font Mono', sample: TERMINAL_ICON_SAMPLE },
  { family: 'Symbols Nerd Font', sample: TERMINAL_ICON_SAMPLE },
] as const;
const TERMINAL_FONT_FAMILY = `${TERMINAL_FONT_LOADS.map(({ family }) => `"${family}"`).join(', ')}, monospace`;

async function ensureTerminalFontLoaded(): Promise<void> {
  if (typeof document === 'undefined') {
    return;
  }

  const fonts = (
    document as Document & {
      fonts?: {
        load?: (font: string, text?: string) => Promise<unknown>;
      };
    }
  ).fonts;

  if (!fonts?.load) {
    return;
  }

  try {
    await Promise.allSettled(
      TERMINAL_FONT_LOADS.map(({ family, sample }) =>
        fonts.load(`${TERMINAL_FONT_SIZE}px "${family}"`, sample)
      )
    );
  } catch {
    // Fall back to terminal creation even if the browser font API fails.
  }
}

export interface TerminalInstanceRuntime {
  terminal: Terminal;
  fitAddon: FitAddon;
}

export async function createTerminalInstance(
  container: HTMLElement
): Promise<TerminalInstanceRuntime> {
  await ensureTerminalInit();
  await ensureTerminalFontLoaded();

  const terminal = new Terminal({
    cursorBlink: true,
    fontSize: TERMINAL_FONT_SIZE,
    fontFamily: TERMINAL_FONT_FAMILY,
    theme: getTerminalTheme(),
  });

  const fitAddon = new FitAddon();

  terminal.loadAddon(fitAddon);
  terminal.open(container);
  fitAddon.fit();

  return { terminal, fitAddon };
}
