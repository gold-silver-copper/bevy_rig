# Rig to Bevy Feature Map

Scope: this map targets the versions pinned in this workspace, `rig-core 0.33.0` and `bevy 0.18.1`.

This is not a claim that Rig and Bevy are isomorphic. They are not.

- Rig provides the AI domain model: providers, models, agents, tools, retrieval, extraction, streaming, telemetry, evals.
- Bevy provides the runtime model: app, plugins, world, entities, components, resources, systems, schedules, messages, observers, tasks, assets, scenes, reflection, states.

The right "union" is therefore: Rig concepts become Bevy data and jobs.

## Core translation rules

- Long-lived, user-visible, addressable domain objects become entities with components.
- Shared runtime services and caches become resources.
- Large or disk-backed blobs become assets behind handles.
- Transient work becomes Bevy messages/events plus job entities.
- Callable tools become registered Bevy systems plus tool metadata entities.
- Workflows become schedules, system sets, or scene-like graphs of entities.
- Persistence and editor-facing config use `Reflect` and optionally `Scene`.

## First principles

The strongest mapping is not "Rig object == Bevy object".

The strongest mapping is:

- Rig semantic nouns map to ECS data.
- Rig execution paths map to Bevy schedules and systems.
- Rig async provider IO maps to Bevy task pools or a Tokio resource.

That gives you a library, not a game.

## Recommended ontology

- Agent = entity
- Session = entity
- Chat message = entity
- Tool = entity definition + registered system
- Provider = entity config + resource-backed live client
- Model = component or asset describing capability + selected backend id
- Context document = entity or asset
- Vector index = resource or backend entity
- Workflow run = job entity
- Stream delta = Bevy message
- Final response = component/resource writeback into session entities

## Rig surface mapped to Bevy

| Rig surface | Closest Bevy primitive | Mapping strength | Recommended shape |
|---|---|---:|---|
| `agent::Agent` | `Entity` + `Component`s + `Children` | Strong | Store preamble, model ref, tool refs, context refs, limits, and runtime state on an agent entity. |
| `agent::AgentBuilder` | `Commands`, bundles, spawn helpers | Strong | Builder methods become spawn/config helpers that write components. |
| `agent` prompt hooks / multi-turn flow | schedules, observers, run conditions | Strong | Represent phases like `Plan`, `CallModel`, `ResolveTools`, `Commit`. |
| `completion::Prompt` / `Chat` / `Completion` | messages/events + systems | Strong | A prompt request is not a component forever; it is a run request flowing through schedules. |
| `completion::CompletionRequestBuilder` | transient job entity + config components | Strong | Build request data in ECS, then hand it to an async execution system. |
| `completion::Message` | persistent chat entities, not Bevy `Message` | Medium | Rig's `Message` is conversation content. Bevy `Message` is transient runtime signaling. Do not collapse them into one type. |
| `completion::ToolDefinition` / provider tool defs | reflected config component or asset | Medium | Good fit for metadata entities; still needs custom schema plumbing. |
| `client::*` provider clients | resources | Strong | Live HTTP clients, auth, model listing handles, and pooled connections should live in resources, not components. |
| `providers::*` | plugins + provider config entities | Strong | Each provider becomes a plugin that registers factories, capabilities, and tool/model adapters. |
| `model::*` / model listing | assets or resources | Medium | Treat model catalogs as provider-owned resources, optionally mirrored onto entities for UI/editing. |
| `embeddings::*` | async tasks + assets/resources | Medium | Embedding generation is a background pipeline; store results as components or assets. |
| `EmbeddingsBuilder` | asset preprocessing / ingestion systems | Medium | Good fit for "document ingest" schedules. |
| `vector_store::*` | resource-backed service, optionally backend entities | Medium | Bevy has no native vector search type. Use a resource for the live index and entities for config/ownership. |
| `VectorStoreIndex` as tool | registered system + lookup resource | Strong | Retrieval can be exposed as a tool system that queries a vector-index resource. |
| `tool::Tool` | registered system (`SystemId`) | Strong | This is the cleanest "tools are systems" mapping: keep metadata on entities and store the callable `SystemId` in a registry resource. |
| `tool::ToolSet` | resource registry | Strong | Registry maps tool name or entity id to `SystemId` and schema metadata. |
| `tool::ToolServer` | dispatcher resource + async task runtime | Strong | Central tool execution service becomes a resource that can run or queue systems. |
| `tool::ToolEmbedding` / dynamic tools | tool entities + embedding/index resource | Medium | Tool retrieval is not native ECS; treat it as semantic search over tool entities. |
| `tool::rmcp` / MCP | plugin + external connection resource | Medium | Model Context Protocol is an integration layer, not a Bevy core concept. |
| `tools::ThinkTool` | hidden/internal registered system | Medium | Can be modeled as a built-in system-backed tool used only by certain agents. |
| `extractor::Extractor` | specialized agent archetype or one-shot system | Strong | Structured extraction is just an agent run with a schema-constrained output path. |
| `pipeline::Op` / `TryOp` | systems, chained system sets, run conditions | Strong | Bevy schedules already model ordered, parallelizable dataflow. |
| `pipeline::parallel!` | Bevy schedule parallelism / task pools | Strong | Parallel ops map naturally to independent systems or background tasks. |
| `pipeline::conditional!` | run conditions, observers, states | Strong | Branching belongs in conditions and state transitions. |
| `streaming::*` | Bevy `Message`s or `Event` observers + task components | Strong | Emit deltas as transient messages; write final aggregated content back to session entities. |
| `transcription::*` | async task + asset output | Strong | Audio in, text out. Store input/output as assets or components and process on background pools. |
| `image_generation::*` | async task + image asset | Strong | Generated images are a natural Bevy asset output. |
| `audio_generation::*` | async task + audio asset/blob | Strong | Same pattern as image generation. |
| `loaders::*` | `AssetServer`, `AssetLoader`, ingest systems | Strong | This is one of the best native matches in Bevy. |
| `telemetry::*` | tracing/logging/diagnostics resources | Medium | Bevy can host telemetry, but Rig's GenAI semantic layer is still custom domain code. |
| `evals::*` | dedicated schedules, states, benchmark resources | Medium | Build an evaluation app mode or sub-app, not ad-hoc systems inside the main interaction loop. |
| `integrations::*` | top-level plugins / alternate app frontends | Medium | CLI and Discord are adapters around the core ECS library. |
| `wasm_compat` | Bevy cross-platform/task abstractions | Strong | Platform conditional async behavior maps well to Bevy's task/runtime abstractions. |
| `one_or_many` / `prelude` / internal helpers | plain Rust support code | None | These are implementation conveniences, not Bevy architecture features. |

## Where the mapping is exact

These Rig ideas fit Bevy unusually well:

- agents as entities
- tool execution as registered systems
- provider/model/runtime caches as resources
- async model calls as task-backed job entities
- workflow phases as schedules and system sets
- dynamic context and tool lookup as resource-backed services
- generated media and ingested documents as assets
- reusable agent graphs as scenes or reflective serialized data

## Where the mapping is not exact

These Rig features do not have a native Bevy equivalent and should stay as your domain layer:

- LLM provider protocols
- completion, embedding, transcription, image, and TTS APIs
- vector similarity search semantics
- JSON schema tool contracts
- GenAI telemetry conventions
- MCP protocol semantics

Bevy can host these features, schedule them, store their state, and visualize them. It does not replace them.

## The most important naming conflict

Rig `completion::Message` and Bevy `Message` are different things.

- Rig message = persisted conversational content.
- Bevy message = transient buffered runtime signal.

Recommended split:

- persist chat history as `ChatMessage` entities under a session entity
- use Bevy `Message`s for `RunAgent`, `ModelChunk`, `ToolInvoked`, `ToolFinished`, `RunFailed`, `RunCommitted`

## "Tools are systems" done correctly

If you want a real Bevy-native tool layer, define each tool in two parts:

1. ECS metadata
   - `ToolDefinitionComponent`
   - `ToolSchemaComponent`
   - `ToolVisibility`
   - `ToolOwner`
2. Executable system registration
   - register a Bevy system
   - store the returned `SystemId` in a `ToolRegistry` resource keyed by tool entity or tool name

That gives you:

- string/schema-facing Rig compatibility
- actual Bevy system execution
- hot-swappable tool registries
- the ability to attach/detach tools from agents as data

## Suggested ECS model for a library

### Persistent entities

- `ProviderEntity`
- `ModelEntity`
- `AgentEntity`
- `ToolEntity`
- `ContextEntity`
- `SessionEntity`
- `ChatMessageEntity`
- `WorkflowNodeEntity`

### Resources

- `ProviderClientRegistry`
- `ToolRegistry`
- `VectorIndexRegistry`
- `RuntimeHandle` or `TokioRuntime`
- `ActiveRuns`
- `Diagnostics`

### Transient messages/events

- `RunAgent`
- `RequestToolCall`
- `ToolCallFinished`
- `ModelStreamChunk`
- `ModelStreamCompleted`
- `RunFailed`
- `PersistAssistantMessage`

### Schedules / sets

- `SyncCatalog`
- `AssembleRequest`
- `DispatchModel`
- `ResolveTools`
- `ApplyStreamDelta`
- `CommitRun`
- `CollectTelemetry`

## Best Bevy feature match for each Rig capability

- Rig agents: `Entity`, `Component`, `Children`
- Rig tools: `App::register_system`, `SystemId`, `World::run_system`
- Rig workflows: `Schedule`, `SystemSet`, run conditions, observers
- Rig streaming: `Message`s or `Event` observers plus async tasks
- Rig async provider calls: `bevy_tasks` pools or a Tokio resource
- Rig loaders: `AssetServer`, `AssetLoader`, `Assets<T>`
- Rig reusable graphs: `Scene`, `DynamicScene`, `Reflect`
- Rig stateful runs: `States`, `NextState`, transition schedules
- Rig structured editable config: `Reflect`, `TypeRegistry`, scene serialization

## Practical conclusion

If you want this to feel like a real Bevy library, the main abstraction should not be `RigAgentInBevy`.

It should be something like:

- "agent ECS"
- "tool system registry"
- "provider plugin"
- "run graph"
- "context asset/index"

Rig should be the execution backend and schema vocabulary.
Bevy should be the data model, scheduler, persistence model, and orchestration runtime.
