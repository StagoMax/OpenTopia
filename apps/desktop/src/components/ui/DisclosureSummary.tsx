import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

export type DisclosureSummaryProps = {
  children: ReactNode;
  className?: string;
  icon?: ReactNode;
};

export function DisclosureSummary({
  children,
  className,
  icon,
}: DisclosureSummaryProps) {
  return (
    <summary
      className={
        className
          ? `ot-disclosure-summary ${className}`
          : "ot-disclosure-summary"
      }
    >
      <span className="ot-disclosure-summary__label">
        {icon}
        <span>{children}</span>
      </span>
      <span aria-hidden="true" className="ot-disclosure-summary__state">
        <span className="ot-disclosure-summary__state-collapsed">展开</span>
        <span className="ot-disclosure-summary__state-expanded">收起</span>
        <ChevronRight
          aria-hidden="true"
          className="ot-disclosure-summary__chevron"
          size={14}
        />
      </span>
    </summary>
  );
}
