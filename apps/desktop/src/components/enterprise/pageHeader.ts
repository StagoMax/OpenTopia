import { useEffect, useRef } from "react";

export type EnterprisePageHeader = {
  title: string;
  backLabel: string;
  onBack(): void;
};

export type EnterprisePageHeaderChange = (
  header: EnterprisePageHeader | null,
) => void;

/**
 * Lets a domain-owned subpage publish its path and back action to the shared
 * workspace header without moving the domain's editor state into App.
 */
export function useEnterpriseSubpageHeader(
  onChange: EnterprisePageHeaderChange | undefined,
  active: boolean,
  header: EnterprisePageHeader,
) {
  const onBackRef = useRef(header.onBack);
  onBackRef.current = header.onBack;

  useEffect(() => {
    if (!onChange) return;
    if (!active) {
      onChange(null);
      return;
    }

    onChange({
      title: header.title,
      backLabel: header.backLabel,
      onBack: () => onBackRef.current(),
    });
    return () => onChange(null);
  }, [active, header.backLabel, header.title, onChange]);
}
