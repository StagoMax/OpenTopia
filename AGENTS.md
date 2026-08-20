# Repository Instructions

## Performance Engineering

- When designing architecture or implementing code, treat performance as a
  first-class requirement. Consider startup and loading time, runtime latency,
  main-thread blocking, memory and resource usage, and interaction smoothness.
- Prefer designs that load only what is needed, avoid unnecessary work and
  repeated computation, and keep expensive operations off latency-sensitive
  paths so the application remains responsive and does not stutter.
- For performance-sensitive changes, identify likely bottlenecks early and
  verify the result with appropriate profiling, measurement, or regression
  tests. Avoid speculative micro-optimizations that reduce clarity without
  evidence of benefit.

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
