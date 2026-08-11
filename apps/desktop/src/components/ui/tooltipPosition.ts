export type TooltipPlacement = "top" | "right" | "bottom" | "left";

export type TooltipRect = {
  top: number;
  right: number;
  bottom: number;
  left: number;
  width: number;
  height: number;
};

export type TooltipViewport = {
  width: number;
  height: number;
};

export type TooltipPosition = {
  left: number;
  top: number;
  placement: TooltipPlacement;
};

function oppositePlacement(placement: TooltipPlacement): TooltipPlacement {
  switch (placement) {
    case "top":
      return "bottom";
    case "right":
      return "left";
    case "bottom":
      return "top";
    case "left":
      return "right";
  }
}

function positionForPlacement(
  trigger: TooltipRect,
  tooltip: Pick<TooltipRect, "width" | "height">,
  placement: TooltipPlacement,
  gap: number,
): Omit<TooltipPosition, "placement"> {
  switch (placement) {
    case "top":
      return {
        left: trigger.left + (trigger.width - tooltip.width) / 2,
        top: trigger.top - tooltip.height - gap,
      };
    case "right":
      return {
        left: trigger.right + gap,
        top: trigger.top + (trigger.height - tooltip.height) / 2,
      };
    case "bottom":
      return {
        left: trigger.left + (trigger.width - tooltip.width) / 2,
        top: trigger.bottom + gap,
      };
    case "left":
      return {
        left: trigger.left - tooltip.width - gap,
        top: trigger.top + (trigger.height - tooltip.height) / 2,
      };
  }
}

export function calculateTooltipPosition(
  trigger: TooltipRect,
  tooltip: Pick<TooltipRect, "width" | "height">,
  viewport: TooltipViewport,
  preferredPlacement: TooltipPlacement,
  gap: number,
  margin: number,
): TooltipPosition {
  const perpendicular: TooltipPlacement[] =
    preferredPlacement === "top" || preferredPlacement === "bottom"
      ? ["right", "left"]
      : ["top", "bottom"];
  const placements = [
    preferredPlacement,
    oppositePlacement(preferredPlacement),
    ...perpendicular,
  ];

  for (const placement of placements) {
    const position = positionForPlacement(trigger, tooltip, placement, gap);
    if (
      position.left >= margin &&
      position.top >= margin &&
      position.left + tooltip.width <= viewport.width - margin &&
      position.top + tooltip.height <= viewport.height - margin
    ) {
      return { ...position, placement };
    }
  }

  const preferred = positionForPlacement(
    trigger,
    tooltip,
    preferredPlacement,
    gap,
  );
  return {
    left: Math.min(
      Math.max(margin, preferred.left),
      Math.max(margin, viewport.width - tooltip.width - margin),
    ),
    top: Math.min(
      Math.max(margin, preferred.top),
      Math.max(margin, viewport.height - tooltip.height - margin),
    ),
    placement: preferredPlacement,
  };
}
