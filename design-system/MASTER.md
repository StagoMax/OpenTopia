# OpenTopia Design System

## Purpose

OpenTopia is a desktop workbench for coding and long-running agent work. Its
visual language is compact, quiet, and utilitarian: a pale neutral sidebar,
white work surface, hairline dividers, and one restrained blue action color.
It is inspired by the information hierarchy of the Codex desktop application,
not a copy of its branding or assets.

This document and `apps/desktop/src/styles/tokens.css` are the source of truth
for all new desktop UI. Read them before adding or changing a visual component.

## Visual Rules

- Work surfaces are white. Application chrome and navigation use
  `--surface-chrome` or `--sidebar-bg`; do not introduce colored page sections.
- Separate workspace regions with `--border`, not shadows. Use a shadow only
  for a popover, modal, or transient floating control.
- Use 4px, 6px, or 8px radii only. Do not use pill shapes except `Badge`.
- Use the 2px spacing scale in `tokens.css`. The normal control height is 32px;
  compact toolbar controls are 28px.
- Use `--font-sans` for application UI and `--font-mono` only for code, paths,
  commands, and data that benefits from alignment.
- Body text is 14px. Compact labels are 12px. Small metadata is 11px. Do not
  add ad-hoc font sizes.
- Icons come only from `lucide-react`. Use 14px, 16px, 18px, or 20px and keep
  icon-only controls accessible with a descriptive `aria-label`.
- Blue is reserved for the primary action, current focus, selected state, and
  meaningful links. Success, warning, and danger colors indicate status only.

## Components

New generic controls must use or extend `apps/desktop/src/components/ui/`:

- `Button`: `primary`, `secondary`, `quiet`, and `danger` variants.
- `IconButton`: an icon-only `Button`; an `aria-label` is required.
- `TextField`: visible label, hint, error state, and matching accessible
  description wiring.
- `Panel`: a compact bordered tool surface, not a page-layout card.
- `Badge`: compact status metadata only.

Compose these primitives before creating a component-specific visual pattern.
If a pattern appears in two features, promote it to `components/ui/` rather
than duplicating CSS.

## Interaction And Accessibility

- All interactive elements keep a visible `--focus-ring` focus state.
- Hover and pressed feedback must not change layout. Use the shared motion
  tokens and respect `prefers-reduced-motion`.
- Buttons use native `button` elements. Do not add click handlers to generic
  `div` or `span` elements.
- Disable controls while their async action is pending and present an explicit
  loading, success, or error state where necessary.
- Never rely only on color for a warning, error, selected state, or progress.

## Implementation Rules

- Add raw color values only in `tokens.css`.
- Add a new token only when no existing semantic token describes the role.
- Do not add an icon library, a font family, gradients, decorative illustrations,
  or arbitrary `z-index` values without an explicit product decision.
- Use the `--z-*` layer scale for fixed, popover, modal, and toast UI.
- Use existing tokens for new CSS. Run `pnpm design:check` before handoff.

## Legacy Migration

Existing feature CSS is legacy and is migrated opportunistically. When editing
an existing feature, replace the touched raw color, spacing, radius, and shadow
values with the closest semantic token. Do not run a mass visual rewrite solely
to remove legacy values.

## Prompt For AI Changes

```text
Read design-system/MASTER.md and apps/desktop/src/styles/tokens.css first.
Use components from apps/desktop/src/components/ui when they fit.
Do not add raw colors, arbitrary font sizes, radii, spacing, z-index values,
new fonts, or a second icon library. Preserve visible focus states and use a
native semantic element for every interactive control.
```
