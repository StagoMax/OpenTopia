import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import { Terminal, type ITheme } from "xterm";
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
      theme: readTerminalTheme(),
      cursorBlink: true,
      cursorStyle: "block",
      cursorInactiveStyle: "outline",
      fontSize: readCssNumber("--font-size-code", 12),
      fontFamily: readCssToken("--font-mono"),
      lineHeight: readCssNumber("--line-height-body", 1.45),
      letterSpacing: 0,
      minimumContrastRatio: 4.5,
      disableStdin: disabled,
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
    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = readTerminalTheme();
      terminal.options.fontFamily = readCssToken("--font-mono");
      terminal.options.fontSize = readCssNumber("--font-size-code", 12);
      fitTerminal();
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });

    return () => {
      resizeObserver.disconnect();
      themeObserver.disconnect();
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

  useEffect(() => {
    if (!terminalRef.current) return;
    terminalRef.current.options.disableStdin = disabled;
    terminalRef.current.options.cursorBlink = !disabled;
  }, [disabled]);

  return (
    <div
      ref={containerRef}
      className="xterm-container"
      role="region"
      aria-label="终端输入区"
      aria-disabled={disabled}
    />
  );
});

function readCssToken(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
}

function readCssNumber(name: string, fallback: number): number {
  return Number.parseFloat(readCssToken(name)) || fallback;
}

function readTerminalTheme(): ITheme {
  return {
    background: readCssToken("--terminal-background"),
    foreground: readCssToken("--terminal-foreground"),
    cursor: readCssToken("--terminal-cursor"),
    cursorAccent: readCssToken("--terminal-background"),
    selectionBackground: readCssToken("--terminal-selection"),
    black: readCssToken("--terminal-ansi-black"),
    brightBlack: readCssToken("--terminal-ansi-bright-black"),
    red: readCssToken("--terminal-ansi-red"),
    brightRed: readCssToken("--terminal-ansi-bright-red"),
    green: readCssToken("--terminal-ansi-green"),
    brightGreen: readCssToken("--terminal-ansi-bright-green"),
    yellow: readCssToken("--terminal-ansi-yellow"),
    brightYellow: readCssToken("--terminal-ansi-bright-yellow"),
    blue: readCssToken("--terminal-ansi-blue"),
    brightBlue: readCssToken("--terminal-ansi-bright-blue"),
    magenta: readCssToken("--terminal-ansi-magenta"),
    brightMagenta: readCssToken("--terminal-ansi-bright-magenta"),
    cyan: readCssToken("--terminal-ansi-cyan"),
    brightCyan: readCssToken("--terminal-ansi-bright-cyan"),
    white: readCssToken("--terminal-ansi-white"),
    brightWhite: readCssToken("--terminal-ansi-bright-white"),
  };
}
