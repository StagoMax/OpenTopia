"""OpenTopia integration for the pinned Harness-Bench checkout.

The adapter itself is imported by the benchmark runner after the pinned
Harness-Bench ``src`` directory has been added to ``sys.path``.  Keeping this
package initializer dependency-free also lets the trace converter be tested
without installing the third-party benchmark package globally.
"""
