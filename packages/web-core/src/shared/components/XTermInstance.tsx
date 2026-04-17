import { useCallback, useEffect, useMemo, useRef } from 'react';

import { useTheme } from '@/shared/hooks/useTheme';
import { createTerminalInstance } from '@/shared/components/xterm-instance-runtime';
import { TERMINAL_SHORTCUTS_ROOT_ATTR } from '@/shared/keyboard/shortcutGuards';
import { getTerminalTheme } from '@/shared/lib/terminalTheme';
import { useTerminal } from '@/shared/hooks/useTerminal';
import type { FitAddon, Terminal } from '@/shared/lib/terminalAdapter';

interface XTermInstanceProps {
  tabId: string;
  workspaceId: string;
  isActive: boolean;
  onClose?: () => void;
}

export function XTermInstance({
  tabId,
  workspaceId,
  isActive,
  onClose,
}: XTermInstanceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const resizeRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const initialSizeRef = useRef({ cols: 80, rows: 24 });
  const { theme } = useTheme();
  const {
    registerTerminalInstance,
    getTerminalInstance,
    createTerminalConnection,
    getTerminalConnection,
  } = useTerminal();

  const endpoint = useMemo(() => {
    const protocol = window.location.protocol === 'https:' ? 'https:' : 'http:';
    const host = window.location.host;
    return `${protocol}//${host}/api/terminal/ws?workspace_id=${workspaceId}&cols=${initialSizeRef.current.cols}&rows=${initialSizeRef.current.rows}`;
  }, [workspaceId]);

  const fitTerminal = useCallback(() => {
    fitAddonRef.current?.fit();
    if (terminalRef.current) {
      const conn = getTerminalConnection(tabId);
      conn?.resize(terminalRef.current.cols, terminalRef.current.rows);
    }
  }, [tabId, getTerminalConnection]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const existing = getTerminalInstance(tabId);
    if (existing) {
      const { terminal, fitAddon } = existing;
      if (terminal.element) {
        container.appendChild(terminal.element);
        fitAddon.fit();
      }
      terminalRef.current = terminal;
      fitAddonRef.current = fitAddon;
      return;
    }

    if (terminalRef.current) return;

    let isCancelled = false;
    let mountedTerminal: Terminal | null = null;

    void (async () => {
      const latestExisting = getTerminalInstance(tabId);
      if (latestExisting) {
        const { terminal, fitAddon } = latestExisting;
        if (!isCancelled && terminal.element) {
          container.appendChild(terminal.element);
          fitAddon.fit();
        }
        terminalRef.current = terminal;
        fitAddonRef.current = fitAddon;
        return;
      }

      const { terminal, fitAddon } = await createTerminalInstance(container);

      if (isCancelled) {
        terminal.dispose();
        return;
      }

      mountedTerminal = terminal;
      initialSizeRef.current = { cols: terminal.cols, rows: terminal.rows };

      terminalRef.current = terminal;
      fitAddonRef.current = fitAddon;

      if (!getTerminalConnection(tabId)) {
        createTerminalConnection(
          tabId,
          endpoint,
          (data: string) => terminal.write(data),
          onClose
        );
      }

      registerTerminalInstance(tabId, terminal, fitAddon);

      terminal.onData((data: string) => {
        const conn = getTerminalConnection(tabId);
        conn?.send(data);
      });
    })();

    return () => {
      isCancelled = true;
      if (mountedTerminal?.element && mountedTerminal.element.parentNode) {
        mountedTerminal.element.parentNode.removeChild(mountedTerminal.element);
      }
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [
    tabId,
    endpoint,
    onClose,
    getTerminalInstance,
    registerTerminalInstance,
    createTerminalConnection,
    getTerminalConnection,
  ]);

  useEffect(() => {
    if (!resizeRef.current) return;
    const observer = new ResizeObserver(fitTerminal);
    observer.observe(resizeRef.current);
    return () => observer.disconnect();
  }, [fitTerminal]);

  useEffect(() => {
    if (isActive) terminalRef.current?.focus();
  }, [isActive]);

  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.theme = getTerminalTheme();
    }
  }, [theme]);

  return (
    <div ref={resizeRef} className="w-full h-full px-2 py-1">
      <div
        ref={containerRef}
        className="w-full h-full"
        {...{ [TERMINAL_SHORTCUTS_ROOT_ATTR]: '' }}
      />
    </div>
  );
}
