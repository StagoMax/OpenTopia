# Incremental workspace build misses transitive invalidation

The workspace builder accepts package definitions
`{ name, dependencies, inputs }` and a previous cache mapping package names to
hashes. It should produce deterministic build waves and rebuild a package when
its own inputs change **or any dependency rebuilds**.

Repair the existing implementation:

- Validate unique package names, existing dependency references, no self edges,
  and acyclic graphs.
- Hash a package from its sorted `inputs` object using SHA-256 without mutating
  callers.
- `planBuild(packages, previousCache)` returns `{ waves, rebuild, cache }`.
- Each wave contains every currently available package, sorted by name.
- `rebuild` is the topological order flattened from waves, filtered to packages
  invalidated directly or transitively.
- `cache` contains every current package hash, with keys sorted by package name.

The public exports in `src/hash.js` and `src/planner.js` must remain available.
