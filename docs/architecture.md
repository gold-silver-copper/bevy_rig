# Architecture

This project is grounded in the vendored upstream source trees under [`third_party/bevy-0.18.1`](/Users/kisaczka/Desktop/code/bevy_rig/third_party/bevy-0.18.1) and [`third_party/rig-core-0.33.0`](/Users/kisaczka/Desktop/code/bevy_rig/third_party/rig-core-0.33.0).

Related documents:

- [`docs/rig_bevy_feature_map.md`](/Users/kisaczka/Desktop/code/bevy_rig/docs/rig_bevy_feature_map.md) for the Rig-to-Bevy surface mapping
- [`docs/bevy_rig_blueprint.md`](/Users/kisaczka/Desktop/code/bevy_rig/docs/bevy_rig_blueprint.md) for the recommended target architecture

The design uses two upstream observations:

- Bevy's example corpus shows the right building blocks for a desktop control plane: entity-centric UI, `Node`-based layouts, button interaction queries, message-driven input, `Children` relationships, and stateful resources.
- Rig's example corpus shows the workload surface to model in ECS: providers, context injection, dynamic context, tools, dynamic tools, agent-as-tool composition, extractors, embeddings, vector search, structured output, loaders, media, and telemetry.

## ECS layout

- Provider entities own provider capability entities.
- Agent entities own agent capability entities and a session entity.
- Session entities own chat message entities.
- Tool entities remain first-class data and are translated into real `ToolDyn` objects only when a run starts.
- Context entities remain first-class data and are attached to agents by entity id.

## Runtime path

The runtime bridge is a Tokio runtime stored as a Bevy resource. When the user runs an agent:

1. The app snapshots the active agent entity, provider entity, tool entities, and context entities.
2. Tool entities are converted into real Rig tools.
3. The selected Rig provider client is instantiated dynamically from the provider entity.
4. A real Rig `AgentBuilder` is assembled from ECS data.
5. The response is sent back through a channel and written into the session as a new assistant message entity.

## Intentional gaps

- Model discovery is only represented as a capability surface today; live model listing is not yet generalized.
- Context entities are injected as static context first. The ECS model already leaves room for embedding-backed dynamic context and vector-store entities.
- Agent-as-tool is modeled as a Rig feature and an agent capability, but not yet wired as a runtime attachment path.

## Why this shape

The goal is not a hardcoded "OpenAI UI". The goal is a Bevy-native graph of entities that can represent:

- any Rig provider
- online and local endpoints
- tool and context attachment as data
- agent construction as data
- future workflow, routing, and telemetry features without rewriting the storage model
