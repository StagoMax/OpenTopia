# Repository Instructions

## Desktop UI

For any change under `apps/desktop/src` that adds or changes visible UI, read
`design-system/MASTER.md` and `apps/desktop/src/styles/tokens.css` before
editing.

- Use primitives from `apps/desktop/src/components/ui/` whenever they fit.
- Do not introduce raw color values, ad-hoc spacing, radius, font-size, or
  `z-index` values in new UI code. Use a semantic token instead.
- Use `lucide-react` as the sole icon library. Icon-only controls require an
  `aria-label`.
- Preserve visible keyboard focus states and use native semantic interactive
  elements.
- Run `pnpm design:check` and the desktop type check for UI changes.

When touching legacy styles, migrate only the declarations in the feature being
changed. Do not perform broad reformatting or a mass visual rewrite.
