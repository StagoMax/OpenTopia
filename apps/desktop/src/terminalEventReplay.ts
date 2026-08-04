export type TerminalEventIdentity = {
  id: string;
};

/**
 * Writes each unseen event only after a terminal target is ready.
 *
 * A terminal can be recreated without the surrounding React state changing
 * (notably during development Strict Mode). Callers provide a fresh `written`
 * set for that terminal generation so the complete event stream is replayed.
 */
export function writePendingTerminalEvents<
  Event extends TerminalEventIdentity,
  Terminal,
>(
  events: readonly Event[],
  terminal: Terminal | null,
  written: Set<string>,
  writeEvent: (event: Event, terminal: Terminal) => void,
): void {
  if (!terminal) return;

  for (const event of events) {
    if (written.has(event.id)) continue;
    writeEvent(event, terminal);
    written.add(event.id);
  }
}
