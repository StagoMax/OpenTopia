# Repository Instructions

## Proportional Problem Solving

- Investigate in proportion to the task's risk and complexity before changing
  code.
- For clearly isolated mechanical changes, copy changes, or simple defects,
  implement directly and run targeted validation.
- For bugs and behavior changes, first make a lightweight check of the likely
  root cause, the module or abstraction that owns the behavior, and any obvious
  callers or parallel implementations that may also be affected.
- Keep the fix local when the evidence shows the problem is isolated. Expand to
  a broader systemic investigation only when there are concrete signals such as
  recurring failures, shared logic, duplicated implementations, patches needed
  in multiple places, or concerns involving state, concurrency, caching,
  lifecycle, performance, or data consistency.
- When systemic signals exist, identify the correct abstraction boundary,
  inspect the affected paths, and prefer fixing the root cause there.
- Do not broaden the change merely for architectural completeness. Report the
  tradeoff briefly only when the chosen approach materially changes scope or
  risk.

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
