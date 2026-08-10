# Disabled workspace plugin still shadows an enabled system plugin

The registry merges `system`, `user`, and `workspace` definitions. Higher scope
normally overrides lower scope, but a disabled higher-scope definition should
remove only that definition and allow the next enabled definition to resolve.
Currently it makes the plugin disappear entirely. Dependency resolution also
loads dependents before dependencies in some merges.

`resolveRegistry(layers)` must return enabled plugins with unique IDs using
precedence `workspace > user > system`, falling back past disabled definitions.
Reject duplicate IDs within one layer, missing dependencies, and cycles. Return
plugins in deterministic dependency-first order; ties use ID. Do not mutate
input layers.
