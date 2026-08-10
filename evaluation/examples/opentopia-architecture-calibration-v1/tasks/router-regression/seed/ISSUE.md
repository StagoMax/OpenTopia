# Router regression: static routes and decoded parameters

After the last matcher refactor, a parameter route registered first can capture a
more specific static route. Encoded path parameters are also returned verbatim,
and malformed percent escapes currently crash request dispatch.

Repair the existing router without changing its public API:

- `Router.add(method, pattern, value)` registers a route and remains chainable.
- `Router.match(method, pathname)` returns `{ value, params }` or `null`.
- Static segments outrank `:parameter` segments, which outrank a terminal
  `*rest` segment, regardless of registration order.
- Parameters and rest captures are decoded with `decodeURIComponent`.
- A malformed escape is a non-match, not an exception.
- Method matching is case-insensitive and query strings are ignored.
- Duplicate parameter names and non-terminal wildcards are rejected at add time.

Do not add dependencies.
