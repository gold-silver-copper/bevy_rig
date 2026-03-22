# bevy_rig Blueprint

Scope: this blueprint is based on the vendored `rig-core 0.33.0` and `bevy 0.18.1` trees in `third_party/`.

This document answers a different question than `rig_bevy_feature_map.md`.

- The feature map asks: "what maps to what?"
- This blueprint asks: "what architecture should we actually build?"

## Primary vendored sources

Rig:

- `third_party/rig-core-0.33.0/README.md`
- `third_party/rig-core-0.33.0/src/lib.rs`
- `third_party/rig-core-0.33.0/src/agent/*`
- `third_party/rig-core-0.33.0/src/completion/*`
- `third_party/rig-core-0.33.0/src/tool/*`
- `third_party/rig-core-0.33.0/src/vector_store/*`
- `third_party/rig-core-0.33.0/src/pipeline/*`
- `third_party/rig-core-0.33.0/src/streaming.rs`
- `third_party/rig-core-0.33.0/src/extractor.rs`
- `third_party/rig-core-0.33.0/src/loaders/*`
- representative examples under `third_party/rig-core-0.33.0/examples/`

Bevy:

- `third_party/bevy-0.18.1/README.md`
- `third_party/bevy-0.18.1/docs/cargo_features.md`
- `third_party/bevy-0.18.1/docs/debugging.md`
- `third_party/bevy-0.18.1/docs/profiling.md`
- `third_party/bevy-0.18.1/examples/README.md`
- representative examples under `third_party/bevy-0.18.1/examples/`

## Source-driven thesis

Reading the vendored sources leads to four design conclusions.

### 1. Rig is the AI vocabulary, not the runtime shell

Rig's README, crate docs, and examples revolve around:

- provider-agnostic clients and models
- agents and agent builders
- tools and dynamic tools
- extraction and structured output
- embeddings, vector stores, and RAG
- streaming and multi-turn loops
- agent orchestration, routing, and parallelization
- media generation, transcription, loaders, telemetry, and MCP

That means Rig should define the domain semantics of `bevy_rig`.

### 2. Bevy is the orchestration shell, not just a renderer

Bevy's vendored README, feature docs, and example corpus emphasize:

- modular plugins
- ECS world data and relationships
- schedules, custom schedules, and run conditions
- messages, observers, and one-shot systems
- async task pools
- assets and asset processing
- reflection, serialization, and scenes
- states and sub-states

That means Bevy should define the storage, scheduling, persistence, and integration model of `bevy_rig`.

### 3. The best `bevy_rig` is headless-first

Bevy's cargo feature docs explicitly separate "full engine" from smaller profiles and collections, including `default_app` and non-rendering use cases.

So the core library should not require the full `bevy` default feature set.

### 4. "Tools are systems" is the strongest architectural bet

Rig tools are callable units with schemas and outputs.
Bevy has a real push-based execution path through registered systems and `SystemId`.

This is the cleanest native bridge between the two libraries.

## Recommended crate layout

The best long-term shape is a small workspace, even if it starts in one crate.

### `bevy_rig_core`

Headless ECS domain and scheduling layer.

Depends on:

- `bevy_app`
- `bevy_ecs`
- `bevy_tasks`
- `bevy_reflect`
- `bevy_state`
- `bevy_asset` optionally
- `bevy_scene` optionally

Responsibilities:

- entities, components, resources, messages
- schedules and system sets
- tool registry and execution model
- workflow graph execution
- persistence schemas

### `bevy_rig_rig`

Rig integration layer.

Depends on:

- `rig-core`
- `bevy_rig_core`

Responsibilities:

- provider plugins
- model factories
- request assembly from ECS state
- stream ingestion from Rig back into ECS
- loader/embedding/vector index bridges

### `bevy_rig_ui`

Optional front-end crate.

Depends on full `bevy` or your chosen UI stack.

Responsibilities:

- inspectors
- session views
- tool/model/context editors
- debug panels

### `bevy_rig_app`

Optional desktop application.

Responsibilities:

- compose plugins
- own app-specific states
- provide local persistence paths
- install diagnostics and profiling defaults

If you stay in one crate for now, keep this separation as module boundaries.

## Recommended dependency strategy

For the core library, prefer Bevy subcrates over the full `bevy` crate.

Why:

- compile time stays lower
- the library remains usable in headless and server contexts
- rendering/UI remains optional
- the architecture stays honest about what the core actually needs

Use the full `bevy` crate only in app or UI crates.

## Architectural layers

### Layer 1: Data model

Persistent world state.

This is where agents, tools, sessions, contexts, providers, models, and workflow nodes live.

### Layer 2: Runtime services

Shared resources and registries.

This is where live provider clients, task pools, vector indexes, tool system IDs, and diagnostics live.

### Layer 3: Execution graph

Messages, job entities, custom schedules, and system sets.

This is where a user intent becomes a Rig request, tool calls, stream deltas, and persisted results.

### Layer 4: Adapters

Rig provider plugins, MCP bridges, asset loaders, UI inspectors, CLI/desktop shells.

## Canonical ECS model

### Persistent entities

- `ProviderEntity`
- `ModelEntity`
- `AgentEntity`
- `ToolEntity`
- `ContextEntity`
- `SessionEntity`
- `ChatMessageEntity`
- `WorkflowNodeEntity`
- `RunEntity`

### Core components

- `Name`
- `ProviderKind`
- `ProviderConfig`
- `ProviderCapabilities`
- `ModelRef`
- `ModelCapabilities`
- `AgentSpec`
- `AgentPreamble`
- `AgentPolicy`
- `AgentToolRefs`
- `AgentContextRefs`
- `SessionOwner`
- `SessionStatus`
- `ChatMessageRole`
- `ChatMessageContent`
- `ChatMessageMetadata`
- `ToolSpec`
- `ToolSchema`
- `ToolKind`
- `ToolBinding`
- `ContextPayload`
- `ContextSource`
- `ContextEmbeddingStatus`
- `WorkflowNodeKind`
- `WorkflowEdges`
- `RunStatus`
- `RunOwner`
- `RunRequest`
- `RunResult`

### Relationships

Use Bevy entity relationships and children aggressively.

Recommended ownership graph:

- provider owns model entities
- agent owns capability and policy entities if needed
- agent owns one or more sessions
- session owns chat messages
- workflow owns workflow nodes

Use plain entity references for many-to-many attachment:

- agent -> tools
- agent -> contexts
- workflow node -> downstream nodes

## Resources

These should be the main shared runtime services.

- `ToolRegistry`
- `ProviderRegistry`
- `ModelRegistry`
- `VectorIndexRegistry`
- `EmbeddingQueue`
- `ActiveRuns`
- `RunTaskRegistry`
- `TypeRegistryBootstrap`
- `PersistenceConfig`
- `TelemetryConfig`
- `DiagnosticsState`
- `TokioRuntime` only if Bevy task pools alone are insufficient

Notes:

- `ToolRegistry` maps tool entity or tool name to Bevy `SystemId` and schema metadata.
- `ProviderRegistry` maps provider kinds to factories capable of building Rig clients/models from ECS config.
- `VectorIndexRegistry` holds live in-memory or backend index handles. Do not try to serialize the live handles themselves.

## Messages and events

Bevy `Message` and Rig `completion::Message` must remain separate.

Use Bevy messages for transient execution signaling.

Recommended message set:

- `RunAgent`
- `CancelRun`
- `AssemblePrompt`
- `DispatchRigRequest`
- `ToolCallRequested`
- `ToolCallCompleted`
- `ToolCallFailed`
- `StreamChunkReceived`
- `StreamCompleted`
- `RunCommitted`
- `RunFailed`
- `EmbedContextRequested`
- `ContextEmbedded`
- `IndexRebuilt`

Use observers when you need push-style reactions to specific transitions.
Use buffered messages when you want deterministic schedule-phase processing.

## Schedule model

Do not keep everything in `Update`.

The Bevy examples around custom schedules, messages, one-shot systems, run conditions, and states point to a cleaner layout.

Recommended schedule families:

- `CatalogSync`
- `Ingestion`
- `RunPreparation`
- `RunExecution`
- `RunCommit`
- `Telemetry`

Within `Main`, order them relative to Bevy's built-ins if needed.

Recommended system sets inside the run path:

- `Validate`
- `ResolveRefs`
- `AssembleRequest`
- `Dispatch`
- `ApplyToolCalls`
- `ApplyStream`
- `Persist`
- `EmitDiagnostics`

## States

Use Bevy states for high-level control flow, not as a substitute for data.

Recommended top-level states:

- `Boot`
- `Idle`
- `Running`
- `Ingesting`
- `Evaluating`
- `Error`

Recommended sub-states for runs:

- `Pending`
- `Planning`
- `AwaitingTools`
- `Streaming`
- `Finalizing`
- `Completed`
- `Failed`
- `Cancelled`

This gives you explicit transitions for UI, diagnostics, and replay.

## Tool architecture

This is the core of the library.

Each tool should exist in two forms.

### 1. Tool as data

Stored on entities:

- name
- description
- input schema
- output shape
- permissions or visibility
- tags for retrieval
- owning plugin/provider/workflow

### 2. Tool as executable system

Stored in `ToolRegistry`:

- `SystemId`
- invocation policy
- argument decoder
- result encoder

### Why this is better than wrapping `ToolDyn` directly everywhere

- system registration is native Bevy
- you can attach and detach tools as ECS data
- one-shot system execution matches tool invocation well
- tools become inspectable, serializable, and indexable
- dynamic tool retrieval can operate on entities without needing live tool instances

### Recommended execution path

1. agent or workflow emits `ToolCallRequested`
2. dispatcher resolves target tool entity
3. dispatcher finds `SystemId` in `ToolRegistry`
4. system is run as a one-shot or queued run
5. result is converted into Rig-compatible content
6. `ToolCallCompleted` is emitted

## Agents and workflows

Rig examples show several families:

- plain agents
- agents with tools
- agents with context
- multi-turn agents
- structured-output agents
- agent-as-tool
- routers
- orchestrators
- parallel evaluators

The right Bevy model is:

- simple agent = single `AgentEntity`
- composite agent = workflow entity with node graph
- router = classifier node plus conditional edges
- orchestrator = workflow entity with planner and worker nodes
- agent-as-tool = tool entity whose implementation delegates to another agent or workflow entity

Do not force every advanced Rig flow into one monolithic `AgentComponent`.

## Retrieval and context model

Rig examples make two patterns clear:

- static context injection
- dynamic context via embeddings and vector search

Use both explicitly.

### Static context

Best represented as:

- context entities with text payloads
- optional asset-backed payloads for large documents
- agent references to selected contexts

### Dynamic context

Best represented as:

- context entities with embedding metadata
- a live vector index resource
- an ingestion schedule that updates indexes
- retrieval systems that return context entity IDs before final prompt assembly

This keeps retrieval explainable and inspectable inside ECS.

## Documents, loaders, and assets

This is one of the strongest Bevy-native matches.

Rig loaders and Bevy assets fit naturally together.

Recommended rule:

- raw document blobs live as assets
- parsed document chunks become context entities
- embedding artifacts live in resources or backend stores

Use Bevy asset patterns for:

- file-backed documents
- processed content
- hot-reload in development
- asset source abstraction

Use Rig loaders as ingestion helpers, not as your long-term persistence layer.

## Streaming model

Rig streaming examples suggest that streaming is not optional if `bevy_rig` wants to feel alive.

Recommended shape:

- background task handles provider stream
- stream deltas are converted into Bevy messages
- main-thread systems append deltas into a `RunEntity` accumulator
- commit phase writes finalized assistant messages into session entities

Recommended transient message types:

- `TextDelta`
- `ReasoningDelta`
- `ToolCallDelta`
- `StreamUsageDelta`
- `FinalResponse`

Recommended persisted output split:

- display-ready assistant message entities
- structured run transcript entities or blobs for replay/debugging

## Structured output and extraction

Rig's extractor and structured-output support should become a first-class mode, not a side API.

Recommended modeling:

- extraction is a workflow kind
- schema target is stored as reflected metadata
- successful extraction writes typed or reflect-serializable entities/resources

Use cases:

- classification
- route decisions
- typed memory updates
- eval judgments
- tool planning outputs

## Reflection, scenes, and persistence

Bevy's reflection and scene examples strongly suggest a persistence strategy.

Recommended split:

- reflect-serializable config for persistent specifications
- scenes or dynamic scenes for whole graphs of entities
- non-serializable runtime handles kept in resources

Persist:

- providers
- models
- agent specs
- tool specs
- workflow graphs
- contexts and metadata

Do not persist:

- live HTTP clients
- live task handles
- open streams
- active vector DB connections

## Async runtime guidance

Prefer Bevy task pools for:

- ingestion
- embedding jobs
- index rebuilds
- file IO
- provider requests when practical

Use an explicit Tokio runtime resource only when required by Rig or external integrations.

Rule of thumb:

- Bevy task pools for internal ECS-facing background work
- Tokio runtime for external async ecosystems that already assume Tokio

Hide that distinction behind resources and systems so agent logic does not care.

## Telemetry and diagnostics

Rig's telemetry support and Bevy's tracing/diagnostic/profiling docs point to a two-layer approach.

### Layer 1: Bevy-native diagnostics

- run counts
- queue sizes
- stream latency
- tool latency
- index size
- active run count

### Layer 2: Rig / GenAI telemetry

- token usage
- provider request metadata
- tool call traces
- model response metadata

Install both.

Also keep explicit debug-friendly entities:

- last request
- last response
- last tool error
- last provider error

## Plugin model

Everything meaningful should arrive through plugins.

Recommended plugin families:

- `BevyRigCorePlugin`
- `BevyRigPersistencePlugin`
- `BevyRigRunPlugin`
- `BevyRigToolPlugin`
- `BevyRigIngestionPlugin`
- `BevyRigTelemetryPlugin`
- `BevyRigProviderOpenAiPlugin`
- `BevyRigProviderAnthropicPlugin`
- `BevyRigProviderOllamaPlugin`
- `BevyRigMcpPlugin`

Each provider plugin should:

- register provider kind metadata
- register client/model factories
- declare supported capabilities
- optionally register provider-hosted tools

## What "best possible" means here

For this library, "best" does not mean "most magical".

It means:

- headless core
- ECS-native data model
- explicit schedules
- strong persistence story
- tool execution as systems
- providers as plugins
- contexts and workflows as inspectable entities
- optional UI, not required UI
- streaming and telemetry as first-class

## Anti-patterns to avoid

- storing live Rig agents directly as your primary ECS state
- calling providers directly from UI systems
- treating Bevy `Message` as a persistence model
- hiding all orchestration inside one giant resource
- forcing all tools through boxed `ToolDyn` without system registration
- requiring full Bevy rendering features for the core library
- collapsing workflows into one flat agent component

## Recommended implementation order

### Phase 1

Build the headless core:

- entities
- components
- resources
- run messages
- tool registry

### Phase 2

Implement tool execution as registered systems.

### Phase 3

Implement provider plugins and request assembly from ECS data.

### Phase 4

Implement streaming, run commit, and session persistence.

### Phase 5

Implement ingestion, embeddings, vector indexes, and dynamic context/tools.

### Phase 6

Implement workflow graphs, routers, orchestrators, and agent-as-tool composition.

### Phase 7

Add UI/editor shells, scene persistence, and richer diagnostics.

## Final recommendation

If you optimize for the vendored Bevy and Rig material instead of fighting it, the best `bevy_rig` is:

- a headless Bevy ECS application framework
- whose executable semantics are powered by Rig
- where agents, tools, contexts, sessions, and workflows are first-class ECS data
- and where tool invocation is fundamentally Bevy system execution

That is the shape with the fewest conceptual compromises and the most room to grow.
