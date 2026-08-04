export type EventSequencePolicy = "contiguous" | "projected";

export function shouldRecoverEventSequenceGap(
  lastSequence: number | undefined,
  nextSequence: number,
  policy: EventSequencePolicy,
): boolean {
  return (
    policy === "contiguous" &&
    lastSequence !== undefined &&
    nextSequence > lastSequence + 1
  );
}
