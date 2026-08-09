# Inventory Reservation Contract

Implement the dependency-free Node.js inventory reservation tool in this workspace.

## Library

`src/inventory.js` must export:

- `normalizeInventory(records)`: validate an array of `{ sku, onHand, reserved }` records. `sku` is a non-empty string and must be unique. Counts are non-negative integers, `reserved` defaults to zero, and it cannot exceed `onHand`. Return records sorted by `sku`, each with `{ sku, onHand, reserved, available }`.
- `planReservations(inventory, orders)`: validate inventory and an array of unique orders shaped `{ id, sku, quantity, priority }`. Quantity is a positive integer and priority is an integer from 0 through 9. Reject unknown SKUs. Process orders by descending priority, then ascending `id`. Allocate as much available stock as possible. Return `{ allocations, inventory }`; each allocation is `{ id, sku, requested, allocated, status }`, where status is `filled`, `partial`, or `backordered`. Returned inventory uses the normalized shape with updated `reserved` and `available` counts.
- `summarizeReservations(plan)`: return `{ orders, filled, partial, backordered, requestedUnits, allocatedUnits, remainingUnits }`.

Do not mutate caller-owned arrays or objects.

## CLI

`node src/cli.js --input <json> --output <json>` reads `{ "inventory": [...], "orders": [...] }`, writes `{ "plan": ..., "summary": ... }` as pretty JSON with a trailing newline, and prints exactly:

`Planned N orders: A units allocated, R units remaining.`

Invalid arguments or data must print a concise error to stderr, exit nonzero, and not write an output file.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
