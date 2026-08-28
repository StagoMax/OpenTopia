# Zero-divisor contract regression

The exported `divide` function currently returns JavaScript's `Infinity` or
`NaN` values when the divisor is zero. The product contract says that `divide`
must reject a zero divisor with a stable, user-facing error instead.

Callers rely on the function remaining synchronous and on valid non-zero
division preserving the current numeric behavior.
