import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import { Terminal } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import "xterm/css/xterm.css";

export type XtermTerminalHandle = {
  write(data: string): void;
  writeln(data: string): void;
  clear(): void;
  focus(): void;
};

type XtermTerminalProps = {
  disabled?: boolean;
  onData?: (data: string) => void;
  onResize?: (cols: number, rows: number) => void;
};

export const XtermTerminal = forwardRef<
  XtermTerminalHandle,
  XtermTerminalProps
>(function XtermTerminal({ disabled = false, onData, onResize }, ref) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const onDataRef = useRef(onData);
  const onResizeRef = useRef(onResize);
  const disabledRef = useRef(disabled);

  onDataRef.current = onData;
  onResizeRef.current = onResize;
  disabledRef.current = disabled;

  useImperativeHandle(
    ref,
    () => ({
      write(data: string) {
        terminalRef.current?.write(data);
      },
      writeln(data: string) {
        terminalRef.current?.writeln(data);
      },
      clear() {
        terminalRef.current?.clear();
      },
      focus() {
        terminalRef.current?.focus();
      },
    }),
    [],
  );

  const fitTerminal = useCallback(() => {
    try {
      fitAddonRef.current?.fit();
    } catch {
      // The terminal can be temporarily hidden while workbench tabs switch.
    }
  }, []);

  useEffect(() => {
    if (!containerRef.current) return;

    const fitAddon = new FitAddon();
    const terminal = new Terminal({
      theme: {
        background: "#1e1e1e",
        foreground: "#d4d4d4",
        cursor: "#d4d4d4",
        selectionBackground: "#264f78",
        black: "#252935",
        red: "#f48771",
        green: "#8bd8bd",
        yellow: "#ffd59a",
        blue: "#7ab7ef",
        magenta: "#c9c3ff",
        cyan: "#4ec9b0",
        white: "#d4d4d4",
      },
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: 13,
      fontFamily: "'Cascadia Code', 'SFMono-Regular', Consolas, monospace",
      scrollback: 10_000,
      convertEol: false,
      cols: 100,
      rows: 24,
    });

    terminal.loadAddon(fitAddon);
    terminal.open(containerRef.current);
    fitAddon.fit();
    terminal.focus();

    const dataDisposable = terminal.onData((data) => {
      if (!disabledRef.current) onDataRef.current?.(data);
    });
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      onResizeRef.current?.(cols, rows);
    });

    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    const resizeObserver = new ResizeObserver(() => fitTerminal());
    resizeObserver.observe(containerRef.current);

    return () => {
      resizeObserver.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;

      // Xterm schedules viewport refreshes internally. Let those finish while
      // its render service is still alive when a tool tab closes quickly.
      window.setTimeout(() => {
        fitAddon.dispose();
        terminal.dispose();
      }, 100);
    };
  }, [fitTerminal]);

  return <div ref={containerRef} className="xterm-container" />;
});
