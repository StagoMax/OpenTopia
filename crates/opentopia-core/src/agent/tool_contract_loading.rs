use super::{
    canonical_json_string, AgentCore, BTreeMap, ProviderLoadedToolContract, ProviderToolCandidate,
    ProviderToolContractLoad, ProviderToolDisclosure, ProviderToolResult, ToolSource, Value,
    PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY,
};

impl AgentCore {
    pub(super) fn restore_loaded_tool_contract(
        &self,
        saved: &ProviderToolCandidate,
        current: &ProviderToolCandidate,
        eligible_by_name: &BTreeMap<String, ProviderToolCandidate>,
    ) -> Option<ProviderToolCandidate> {
        let loaded = saved.loaded_contract.as_ref()?;
        let loader = eligible_by_name.get(&loaded.loader_name)?;
        if self
            .tool_host
            .catalog
            .provider_contract_loader(&current.name)
            .as_deref()
            != Some(loaded.loader_name.as_str())
        {
            return None;
        }
        let loader_source = self.tool_host.catalog.source(&loader.name)?;
        let target_source = self.tool_host.catalog.source(&current.name)?;
        if matches!(loader_source, ToolSource::Mcp)
            || loader_source != target_source
            || loaded.loader_input_schema_fingerprint
                != provider_tool_schema_fingerprint(&loader.input_schema)
            || loaded.target_base_input_schema_fingerprint
                != provider_tool_schema_fingerprint(&current.input_schema)
            || !provider_tool_schema_has_object_root(&saved.input_schema)
        {
            return None;
        }

        let mut restored = current.clone();
        restored.input_schema = saved.input_schema.clone();
        restored.disclosure = ProviderToolDisclosure::Direct;
        restored.namespace = None;
        restored.loaded_contract = Some(loaded.clone());
        Some(restored)
    }

    pub(super) fn apply_loaded_tool_contracts_from_result(
        &self,
        result: &mut ProviderToolResult,
        exposed: &mut Vec<ProviderToolCandidate>,
    ) -> bool {
        // Contract loads are transient harness control data. Consume them
        // before the result is appended to the next model round so the model
        // receives the schema once, as the actual tool contract.
        let contract_loads = result
            .metadata
            .as_object_mut()
            .and_then(|metadata| metadata.remove(PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY));
        if result.is_error {
            return false;
        }
        let Some(loads) = contract_loads
            .and_then(|value| serde_json::from_value::<Vec<ProviderToolContractLoad>>(value).ok())
        else {
            return false;
        };

        let eligible = self.eligible_provider_tool_candidates();
        let Some(loader) = eligible
            .iter()
            .find(|candidate| candidate.name == result.name)
        else {
            return false;
        };
        let Some(loader_source) = self.tool_host.catalog.source(&loader.name) else {
            return false;
        };
        if matches!(loader_source, ToolSource::Mcp) {
            return false;
        }

        let mut changed = false;
        for load in loads {
            if !provider_tool_schema_has_object_root(&load.input_schema) {
                continue;
            }
            let Some(base) = eligible
                .iter()
                .find(|candidate| candidate.name == load.name)
            else {
                continue;
            };
            if self
                .tool_host
                .catalog
                .provider_contract_loader(&base.name)
                .as_deref()
                != Some(result.name.as_str())
            {
                continue;
            }
            if self.tool_host.catalog.source(&base.name).as_ref() != Some(&loader_source) {
                continue;
            }
            let mut loaded = base.clone();
            loaded.input_schema = load.input_schema;
            loaded.disclosure = ProviderToolDisclosure::Direct;
            loaded.namespace = None;
            loaded.loaded_contract = Some(ProviderLoadedToolContract {
                loader_name: result.name.clone(),
                loader_input_schema_fingerprint: provider_tool_schema_fingerprint(
                    &loader.input_schema,
                ),
                target_base_input_schema_fingerprint: provider_tool_schema_fingerprint(
                    &base.input_schema,
                ),
            });
            if let Some(existing) = exposed
                .iter_mut()
                .find(|candidate| candidate.name == loaded.name)
            {
                if existing != &loaded {
                    *existing = loaded;
                    changed = true;
                }
            } else {
                exposed.push(loaded);
                changed = true;
            }
        }
        changed
    }
}

fn provider_tool_schema_has_object_root(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object.get("type").is_some_and(schema_type_includes_object)
        || object.get("properties").is_some_and(Value::is_object)
    {
        return true;
    }
    ["oneOf", "anyOf"].iter().any(|keyword| {
        object
            .get(*keyword)
            .and_then(Value::as_array)
            .is_some_and(|branches| {
                !branches.is_empty() && branches.iter().all(provider_tool_schema_has_object_root)
            })
    })
}

fn schema_type_includes_object(value: &Value) -> bool {
    value.as_str() == Some("object")
        || value
            .as_array()
            .is_some_and(|types| types.iter().any(|kind| kind.as_str() == Some("object")))
}

fn provider_tool_schema_fingerprint(schema: &Value) -> String {
    crate::model_context::content_fingerprint(canonical_json_string(schema).as_bytes())
}
