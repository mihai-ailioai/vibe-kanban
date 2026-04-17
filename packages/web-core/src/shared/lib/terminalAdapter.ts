import { init } from 'ghostty-web';

export { FitAddon, Terminal } from 'ghostty-web';
export type { ITheme } from 'ghostty-web';

let terminalInitPromise: Promise<void> | null = null;

export function ensureTerminalInit(): Promise<void> {
  if (terminalInitPromise) {
    return terminalInitPromise;
  }

  terminalInitPromise = init().catch((error) => {
    terminalInitPromise = null;
    throw error;
  });

  return terminalInitPromise;
}
