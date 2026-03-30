use anyhow::{Result, anyhow};
use schemars::Schema;
use serde_json::Value;

use crate::{
    catalog::{NodeId, NodeType, NodeValue, PortType, ToolChoiceSetting},
    document::GraphDocument,
    providers::ProviderRegistry,
    runtime::CompiledAgentRun,
};

pub fn compile_agent_run(
    document: &GraphDocument,
    providers: &ProviderRegistry,
    agent_id: NodeId,
) -> Result<CompiledAgentRun> {
    let output_node = document
        .output_targets(agent_id, PortType::TextResponse)
        .into_iter()
        .find(|node_id| {
            matches!(
                document.node(*node_id).map(|node| node.node_type),
                Some(NodeType::TextOutput)
            )
        })
        .ok_or_else(|| anyhow!("connect the Agent output to a Text Output node"))?;

    let model_node = required_source(document, agent_id, PortType::Model, "model")?;
    let (provider_id, model) = match document.node(model_node).map(|node| &node.value) {
        Some(NodeValue::Model {
            provider_id,
            model_name,
        }) => {
            let provider_id = provider_id
                .clone()
                .ok_or_else(|| anyhow!("select a provider inside the Model node"))?;
            let model = model_name.trim();
            if model.is_empty() {
                return Err(anyhow!("enter or select a model inside the Model node"));
            }
            (provider_id, model.to_string())
        }
        _ => return Err(anyhow!("the Agent model input must come from a Model node")),
    };
    let provider = providers
        .provider(&provider_id)
        .cloned()
        .ok_or_else(|| anyhow!("selected provider registration no longer exists"))?;
    if let Some(error) = provider.config_error() {
        return Err(anyhow!(
            "{} is not configured: {error}",
            provider.display_name()
        ));
    }
    if !provider.status.kind.is_ready() {
        return Err(anyhow!(
            "{} is not ready: {}",
            provider.display_name(),
            provider.status.detail
        ));
    }

    let prompt_node = required_source(document, agent_id, PortType::Prompt, "prompt")?;
    let prompt = match document.node(prompt_node).map(|node| &node.value) {
        Some(NodeValue::Text(text)) if !text.trim().is_empty() => text.clone(),
        Some(NodeValue::Text(_)) => return Err(anyhow!("the Prompt node is empty")),
        _ => return Err(anyhow!("the Agent prompt input must come from a Text node")),
    };

    let agent_name = optional_text_source(document, agent_id, PortType::AgentName)?;
    let description = optional_text_source(document, agent_id, PortType::AgentDescription)?;
    let preamble = optional_text_source(document, agent_id, PortType::Preamble)?;
    let static_context = multi_text_sources(document, agent_id, PortType::StaticContext)?;
    let temperature = optional_temperature_source(document, agent_id)?;
    let max_tokens = optional_max_tokens_source(document, agent_id)?;
    let additional_params = optional_json_source(document, agent_id)?;
    let tool_choice = optional_tool_choice_source(document, agent_id)?;
    let default_max_turns = optional_default_max_turns_source(document, agent_id)?;
    let output_schema = optional_schema_source(document, agent_id)?;

    let mut warnings = Vec::new();
    if !document
        .input_sources(agent_id, PortType::ToolServerHandle)
        .is_empty()
    {
        warnings.push("tool_server_handle nodes are stored but not executable in this MVP".into());
    }
    if !document
        .input_sources(agent_id, PortType::DynamicContext)
        .is_empty()
    {
        warnings.push("dynamic_context nodes are stored but not executable in this MVP".into());
    }
    if !document.input_sources(agent_id, PortType::Hook).is_empty() {
        warnings.push("hook nodes are stored but not executable in this MVP".into());
    }
    Ok(CompiledAgentRun {
        agent_id,
        output_node,
        provider,
        model,
        prompt,
        agent_name,
        description,
        preamble,
        static_context,
        temperature,
        max_tokens,
        additional_params,
        tool_choice,
        default_max_turns,
        output_schema,
        warnings,
    })
}

fn required_source(
    document: &GraphDocument,
    agent_id: NodeId,
    ty: PortType,
    label: &str,
) -> Result<NodeId> {
    document
        .input_sources(agent_id, ty)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Agent is missing a required {label} node"))
}

fn optional_text_source(
    document: &GraphDocument,
    agent_id: NodeId,
    ty: PortType,
) -> Result<Option<String>> {
    let Some(source) = document.input_sources(agent_id, ty).into_iter().next() else {
        return Ok(None);
    };

    let value = match document.node(source).map(|node| &node.value) {
        Some(NodeValue::Text(value)) => value.trim().to_string(),
        _ => {
            return Err(anyhow!(
                "connected node type does not match expected text field"
            ));
        }
    };

    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn multi_text_sources(
    document: &GraphDocument,
    agent_id: NodeId,
    ty: PortType,
) -> Result<Vec<String>> {
    let mut values = Vec::new();
    for source in document.input_sources(agent_id, ty) {
        match document.node(source).map(|node| &node.value) {
            Some(NodeValue::Text(value)) if !value.trim().is_empty() => {
                values.push(value.trim().to_string());
            }
            Some(NodeValue::Text(_)) => {}
            _ => return Err(anyhow!("connected node type does not match static context")),
        }
    }
    Ok(values)
}

fn optional_temperature_source(document: &GraphDocument, agent_id: NodeId) -> Result<Option<f64>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::Temperature)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::Temperature(value)) => Ok(Some(*value)),
        _ => Err(anyhow!(
            "temperature input must come from a Temperature node"
        )),
    }
}

fn optional_max_tokens_source(document: &GraphDocument, agent_id: NodeId) -> Result<Option<u64>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::MaxTokens)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::U64(value)) => Ok(Some(*value)),
        _ => Err(anyhow!("max_tokens input must come from a U64 node")),
    }
}

fn optional_json_source(document: &GraphDocument, agent_id: NodeId) -> Result<Option<Value>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::AdditionalParams)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::AdditionalParams(value)) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                let parsed = serde_json::from_str(value)
                    .map_err(|error| anyhow!("Additional Params is not valid JSON: {error}"))?;
                Ok(Some(parsed))
            }
        }
        _ => Err(anyhow!(
            "additional_params input must come from an Additional Params node"
        )),
    }
}

fn optional_tool_choice_source(
    document: &GraphDocument,
    agent_id: NodeId,
) -> Result<Option<ToolChoiceSetting>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::ToolChoice)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::ToolChoice(value)) => Ok(Some(*value)),
        _ => Err(anyhow!(
            "tool_choice input must come from a Tool Choice node"
        )),
    }
}

fn optional_default_max_turns_source(
    document: &GraphDocument,
    agent_id: NodeId,
) -> Result<Option<usize>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::DefaultMaxTurns)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::U64(value)) => Ok(Some(*value as usize)),
        _ => Err(anyhow!("default_max_turns input must come from a U64 node")),
    }
}

fn optional_schema_source(document: &GraphDocument, agent_id: NodeId) -> Result<Option<Schema>> {
    let Some(source) = document
        .input_sources(agent_id, PortType::OutputSchema)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match document.node(source).map(|node| &node.value) {
        Some(NodeValue::OutputSchema(value)) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                let parsed = serde_json::from_str(value)
                    .map_err(|error| anyhow!("Output Schema is not valid JSON: {error}"))?;
                Ok(Some(parsed))
            }
        }
        _ => Err(anyhow!(
            "output_schema input must come from an Output Schema node"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        ApiKeyProviderConfig, OpenAiVariant, ProviderConfig, ProviderKind, ProviderRegistry,
        ProviderStatus, ProviderVariant,
    };

    fn test_registry() -> ProviderRegistry {
        let unique = format!(
            "compile-providers-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        let mut registry = ProviderRegistry::load_or_seed(path);
        let provider_id = registry.first_provider_id().expect("default provider");
        let provider = registry
            .provider_mut(&provider_id)
            .expect("default provider should exist");
        provider.status = ProviderStatus::ready("ready");
        provider.cached_models = vec!["llama3.2:latest".into()];
        registry.touch();
        registry
    }

    #[test]
    fn compile_rejects_missing_provider_selection() {
        let registry = test_registry();
        let mut graph = GraphDocument::demo();
        let model_node = graph
            .first_node_id_by_type(NodeType::Model)
            .expect("demo graph should have a model node");
        let agent_id = graph
            .first_node_id_by_type(NodeType::Agent)
            .expect("demo graph should have an agent");
        graph.set_model_provider(model_node, None);

        let error =
            compile_agent_run(&graph, &registry, agent_id).expect_err("compile should fail");
        assert!(error.to_string().contains("select a provider"));
    }

    #[test]
    fn compile_rejects_unready_provider() {
        let mut registry = test_registry();
        let provider_id = registry.first_provider_id().expect("default provider");
        let provider = registry
            .provider_mut(&provider_id)
            .expect("default provider should exist");
        provider.status = ProviderStatus::error("endpoint unavailable");
        registry.touch();

        let mut graph = GraphDocument::demo();
        graph.apply_provider_registry(&registry);
        let agent_id = graph
            .first_node_id_by_type(NodeType::Agent)
            .expect("demo graph should have an agent");

        let error =
            compile_agent_run(&graph, &registry, agent_id).expect_err("compile should fail");
        assert!(error.to_string().contains("not ready"));
    }

    #[test]
    fn compile_captures_provider_variant() {
        let mut registry = test_registry();
        let openai_id = registry.add_provider(ProviderKind::OpenAi);
        let provider = registry
            .provider_mut(&openai_id)
            .expect("new provider should exist");
        provider.variant = ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi);
        provider.config = ProviderConfig::OpenAi(ApiKeyProviderConfig {
            api_key: "test-key".into(),
            base_url: None,
        });
        provider.status = ProviderStatus::ready("ready");
        provider.cached_models = vec!["gpt-4o-mini".into()];
        registry.touch();

        let mut graph = GraphDocument::demo();
        graph.apply_provider_registry(&registry);
        let model_node = graph
            .first_node_id_by_type(NodeType::Model)
            .expect("demo graph should have a model node");
        let agent_id = graph
            .first_node_id_by_type(NodeType::Agent)
            .expect("demo graph should have an agent");
        graph.set_model_provider(model_node, Some(openai_id.clone()));
        graph.set_node_inline_value_live(model_node, "gpt-4o-mini");

        let compiled =
            compile_agent_run(&graph, &registry, agent_id).expect("compile should succeed");
        assert_eq!(compiled.provider.id, openai_id);
        assert_eq!(
            compiled.provider.variant,
            ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi)
        );
    }
}
