use crate::provider::{CompiledToolContract, ProviderToolCandidate};
use crate::settings::{ProviderFeatureSupport, ProviderToolProtocolCapabilities};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(in crate::provider) const PORTABLE_APPLY_PATCH_DESCRIPTION: &str = "Apply workspace edits by passing exactly one JSON field named `patch`. The value must be a `*** Begin Patch` envelope ending with `*** End Patch`. For updates, use `*** Update File: relative/path` followed by one or more unified `@@` hunks. Do not send `path`, `diff`, or `operation` as separate fields, and do not use bare `SEARCH:`/`REPLACE:` labels.";

pub(in crate::provider) fn portable_function_tool_candidate(
    candidate: &ProviderToolCandidate,
) -> ProviderToolCandidate {
    if candidate.name != "apply_patch" {
        return candidate.clone();
    }
    ProviderToolCandidate {
        name: candidate.name.clone(),
        description: PORTABLE_APPLY_PATCH_DESCRIPTION.to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "A complete *** Begin Patch ... *** End Patch envelope."
                }
            },
            "required": ["patch"],
            "additionalProperties": false
        }),
        disclosure: candidate.disclosure,
        namespace: candidate.namespace.clone(),
    }
}

pub(in crate::provider) struct CompiledOpenAiFunctionCandidate {
    pub(in crate::provider) candidate: ProviderToolCandidate,
    pub(in crate::provider) contract: CompiledToolContract,
    pub(in crate::provider) strict: bool,
}

pub(in crate::provider) fn compile_openai_function_candidate(
    candidate: &ProviderToolCandidate,
    capabilities: ProviderToolProtocolCapabilities,
) -> CompiledOpenAiFunctionCandidate {
    let candidate = portable_function_tool_candidate(candidate);
    let logical_input_schema = candidate.input_schema.clone();

    // A number of OpenAI-compatible endpoints require function parameters to
    // be a single object schema and reject a discriminated union at the root.
    // Advertise a widened object-shaped wire contract for those tools, then
    // retain the original union as the logical contract used by the runtime.
    let portable_root_schema = openai_non_strict_root_object_schema(&logical_input_schema);
    let strict_schema = if portable_root_schema.is_none()
        && capabilities.strict_function_tools == ProviderFeatureSupport::Supported
    {
        openai_strict_function_schema(&logical_input_schema)
    } else {
        None
    };
    let (wire_input_schema, strict) = if let Some(schema) = portable_root_schema {
        (schema, false)
    } else if let Some(schema) = strict_schema {
        (schema, true)
    } else {
        (logical_input_schema.clone(), false)
    };
    let contract = CompiledToolContract {
        name: candidate.name.clone(),
        logical_input_schema,
        wire_input_schema,
    };

    CompiledOpenAiFunctionCandidate {
        candidate,
        contract,
        strict,
    }
}

/// Lowers a root discriminated union to the object-only shape accepted by the
/// broadest OpenAI-compatible function-tool implementations. The result is a
/// deliberately non-strict wire schema: fields required by only one action
/// become optional, while the original union remains the logical validator.
pub(in crate::provider) fn openai_non_strict_root_object_schema(schema: &Value) -> Option<Value> {
    let root = schema.as_object()?;
    let (union_keyword, branches) = match (
        root.get("oneOf").and_then(Value::as_array),
        root.get("anyOf").and_then(Value::as_array),
    ) {
        (Some(branches), None) => ("oneOf", branches),
        (None, Some(branches)) => ("anyOf", branches),
        _ => return None,
    };
    let discriminator = discriminated_union_key(branches)?;

    let mut lowered = root.clone();
    lowered.remove(union_keyword);
    lowered.insert("type".to_string(), json!("object"));
    let mut properties = lowered
        .remove("properties")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut required = lowered
        .remove("required")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut common_branch_required: Option<Vec<Value>> = None;
    let mut discriminator_values = Vec::with_capacity(branches.len());
    let mut discriminator_schemas = Vec::with_capacity(branches.len());
    let branches_reject_unknown_fields = branches
        .iter()
        .all(|branch| branch.get("additionalProperties").and_then(Value::as_bool) == Some(false));

    for branch in branches {
        let branch = branch.as_object()?;
        let branch_is_object = branch.get("type").is_some_and(schema_type_includes_object)
            || branch.get("properties").is_some();
        if !branch_is_object {
            return None;
        }
        let branch_properties = branch.get("properties")?.as_object()?;
        let branch_required = branch.get("required")?.as_array()?;
        match &mut common_branch_required {
            Some(common) => common.retain(|name| branch_required.contains(name)),
            None => common_branch_required = Some(branch_required.clone()),
        }

        let discriminator_schema = branch_properties.get(&discriminator)?;
        let discriminator_value = schema_singleton_value(discriminator_schema)?.clone();
        discriminator_values.push(discriminator_value);
        discriminator_schemas.push(discriminator_schema);

        for (name, property_schema) in branch_properties {
            if name == &discriminator {
                continue;
            }
            if let Some(existing) = properties.get_mut(name) {
                merge_non_strict_property_schema(existing, property_schema);
            } else {
                properties.insert(name.clone(), property_schema.clone());
            }
        }
    }

    let mut discriminator_schema = properties
        .remove(&discriminator)
        .or_else(|| {
            discriminator_schemas
                .first()
                .map(|schema| (*schema).clone())
        })?
        .as_object()
        .cloned()?;
    discriminator_schema.remove("const");
    discriminator_schema.insert("enum".to_string(), Value::Array(discriminator_values));
    properties.insert(discriminator.clone(), Value::Object(discriminator_schema));

    for name in common_branch_required.unwrap_or_default() {
        if !required.contains(&name) {
            required.push(name);
        }
    }
    if required.is_empty() {
        lowered.remove("required");
    } else {
        lowered.insert("required".to_string(), Value::Array(required.clone()));
    }
    let required_names = required
        .iter()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();
    for (name, property_schema) in &mut properties {
        if !required_names.contains(name.as_str()) {
            make_openai_schema_nullable(property_schema)?;
        }
    }
    lowered.insert("properties".to_string(), Value::Object(properties));
    if !lowered.contains_key("additionalProperties") && branches_reject_unknown_fields {
        lowered.insert("additionalProperties".to_string(), Value::Bool(false));
    }
    Some(Value::Object(lowered))
}

fn merge_non_strict_property_schema(existing: &mut Value, incoming: &Value) {
    if existing == incoming {
        return;
    }
    let previous = std::mem::take(existing);
    let mut variants = Vec::new();
    append_non_strict_schema_variants(&mut variants, previous);
    append_non_strict_schema_variants(&mut variants, incoming.clone());
    *existing = json!({ "anyOf": variants });
}

fn append_non_strict_schema_variants(variants: &mut Vec<Value>, schema: Value) {
    if let Some(any_of) = schema
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("anyOf"))
        .and_then(Value::as_array)
    {
        for variant in any_of {
            if !variants.contains(variant) {
                variants.push(variant.clone());
            }
        }
    } else if !variants.contains(&schema) {
        variants.push(schema);
    }
}

/// Lowers the provider-neutral Draft 7 schema into the conservative subset
/// accepted by OpenAI strict function tools. Failure is per tool: callers keep
/// the original schema and send `strict: false` rather than weakening every
/// function definition for the connection.
pub(in crate::provider) fn openai_strict_function_schema(schema: &Value) -> Option<Value> {
    let mut lowered = schema.clone();
    lower_openai_strict_schema_node(&mut lowered)?;
    let root = lowered.as_object()?;
    let root_is_object = root.get("type").is_some_and(schema_type_includes_object)
        || root.get("properties").is_some();
    root_is_object.then_some(lowered)
}

pub(in crate::provider) fn schema_type_includes_object(value: &Value) -> bool {
    value.as_str() == Some("object")
        || value
            .as_array()
            .is_some_and(|types| types.iter().any(|kind| kind.as_str() == Some("object")))
}

pub(in crate::provider) fn lower_openai_strict_schema_node(schema: &mut Value) -> Option<()> {
    let object = schema.as_object_mut()?;
    for annotation in ["$schema", "title", "default", "examples", "deprecated"] {
        object.remove(annotation);
    }
    if let Some(branches) = object.remove("oneOf") {
        let branches = branches.as_array()?;
        if discriminated_union_key(branches).is_none() {
            return None;
        }
        object.insert("anyOf".to_string(), Value::Array(branches.clone()));
    }
    if object.keys().any(|keyword| {
        matches!(
            keyword.as_str(),
            "$ref"
                | "$defs"
                | "definitions"
                | "oneOf"
                | "allOf"
                | "not"
                | "if"
                | "then"
                | "else"
                | "patternProperties"
                | "unevaluatedProperties"
                | "dependentSchemas"
                | "dependencies"
        )
    }) {
        return None;
    }

    if let Some(branches) = object.get_mut("anyOf") {
        for branch in branches.as_array_mut()? {
            lower_openai_strict_schema_node(branch)?;
        }
    }
    if let Some(items) = object.get_mut("items") {
        lower_openai_strict_schema_node(items)?;
    }

    // A root or nested object union owns its properties inside mutually
    // exclusive branches. Adding an empty `properties` map plus
    // `additionalProperties: false` to the union container would reject every
    // field accepted by those branches.
    if object.contains_key("anyOf") && !object.contains_key("properties") {
        return Some(());
    }

    let is_object = object.get("type").is_some_and(schema_type_includes_object)
        || object.get("properties").is_some();
    if !is_object {
        return Some(());
    }

    let originally_required = object
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let properties = object
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;
    let property_names = properties.keys().cloned().collect::<Vec<_>>();
    for (name, property_schema) in properties.iter_mut() {
        lower_openai_strict_schema_node(property_schema)?;
        if !originally_required.contains(name) {
            make_openai_schema_nullable(property_schema)?;
        }
    }
    object.insert("required".to_string(), json!(property_names));
    object.insert("additionalProperties".to_string(), Value::Bool(false));
    Some(())
}

/// A tagged union whose branches require distinct constant values is already
/// mutually exclusive. OpenAI strict tools accept `anyOf` but not `oneOf`, so
/// this proof lets the provider adapter lower the spelling without weakening
/// the provider-neutral contract.
pub(in crate::provider) fn discriminated_union_key(branches: &[Value]) -> Option<String> {
    let first = branches.first()?.as_object()?;
    let first_required = first.get("required")?.as_array()?;
    let first_properties = first.get("properties")?.as_object()?;

    first_required
        .iter()
        .filter_map(Value::as_str)
        .find(|candidate| {
            let mut seen = Vec::<Value>::new();
            for branch in branches {
                let Some(branch) = branch.as_object() else {
                    return false;
                };
                let Some(required) = branch.get("required").and_then(Value::as_array) else {
                    return false;
                };
                if !required
                    .iter()
                    .any(|value| value.as_str() == Some(*candidate))
                {
                    return false;
                }
                let Some(value) = branch
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get(*candidate))
                    .and_then(schema_singleton_value)
                else {
                    return false;
                };
                if seen.contains(value) {
                    return false;
                }
                seen.push(value.clone());
            }
            first_properties
                .get(*candidate)
                .and_then(schema_singleton_value)
                .is_some()
        })
        .map(str::to_string)
}

pub(in crate::provider) fn schema_singleton_value(schema: &Value) -> Option<&Value> {
    schema.get("const").or_else(|| {
        let values = schema.get("enum")?.as_array()?;
        (values.len() == 1).then(|| &values[0])
    })
}

pub(in crate::provider) fn make_openai_schema_nullable(schema: &mut Value) -> Option<()> {
    let object = schema.as_object_mut()?;
    if object.get("type").is_some_and(|kind| {
        kind.as_str() == Some("null")
            || kind
                .as_array()
                .is_some_and(|types| types.iter().any(|value| value.as_str() == Some("null")))
    }) || object
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| branch.get("type").and_then(Value::as_str) == Some("null"))
        })
    {
        return Some(());
    }

    if object.contains_key("const") {
        let original = std::mem::take(schema);
        *schema = json!({ "anyOf": [original, { "type": "null" }] });
        return Some(());
    }
    if let Some(kind) = object.get_mut("type") {
        match kind {
            Value::String(existing) => {
                *kind = json!([existing.clone(), "null"]);
            }
            Value::Array(types) => types.push(Value::String("null".to_string())),
            _ => return None,
        }
        if let Some(values) = object.get_mut("enum").and_then(Value::as_array_mut) {
            values.push(Value::Null);
        }
        return Some(());
    }
    if let Some(branches) = object.get_mut("anyOf").and_then(Value::as_array_mut) {
        branches.push(json!({ "type": "null" }));
        return Some(());
    }

    let original = std::mem::take(schema);
    *schema = json!({ "anyOf": [original, { "type": "null" }] });
    Some(())
}
