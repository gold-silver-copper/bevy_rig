# bevy_rig

`bevy_rig` is a Bevy ECS integration layer for [Rig](https://crates.io/crates/rig-core).

It models providers, models, agents, tools, contexts, runs, sessions, and workflows as Bevy
entities and components, then supplies systems for turning those entities into executable Rig
requests.

The crate is aimed at applications that want to keep AI orchestration inside Bevy's ECS instead of
building a separate runtime around ad hoc structs and service layers.

## What it provides

- provider and model registries as ECS data
- agent, tool, and context entities
- session and transcript persistence inside the world
- run preparation, execution, streaming, and commit system sets
- workflow graph entities and execution helpers
- diagnostics helpers for inspecting runtime state

## Installation

```toml
[dependencies]
bevy_app = "0.18.1"
bevy_ecs = "0.18.1"
bevy_tasks = "0.18.1"
bevy_rig = "0.1.0"
```

Optional features:

- `media`: enables Rig audio, image, and PDF support
- `mcp`: enables Rig MCP support
- `experimental`: enables Rig experimental APIs

## Quick start

```rust
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(RunExecution, complete_echo_runs.in_set(RunExecutionSystems));

    let agent = {
        let world = app.world_mut();
        spawn_agent(world, AgentSpec::new("echo-agent", "mock-echo")).agent
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "hello from bevy_rig"));
    app.update();
}

fn complete_echo_runs(
    mut commands: Commands,
    runs: Query<(Entity, &RunRequest, &RunStatus), With<Run>>,
) {
    for (run, request, status) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        mark_run_completed(&mut commands, run, format!("Echo: {}", request.prompt));
    }
}
```

## Core concepts

### Providers and models

Providers and models are spawned as entities and indexed through registries. Agents bind to models
through component references instead of direct ownership.

### Agents and contexts

Agents, tools, and context documents are separate entities. This makes it easy to inspect,
retarget, attach, detach, and query them through normal ECS systems.

### Runs and sessions

Runs represent individual execution requests. Sessions and chat messages persist conversation state
inside the Bevy world.

### Workflows

Workflows are graph-shaped ECS data. Nodes, edges, and execution state are all represented with
components and events, which makes them easy to debug and visualize.

## Examples

The crate includes small headless examples under [`examples/`](examples):

- `headless_echo`
- `provider_models`
- `tool_dispatch`
- `workflow_graph`
- `workflow_execution`
- `rig_provider_run`
- `rig_provider_workflow`

Run one with:

```bash
cargo run --example headless_echo
```

## Current scope

`bevy_rig` focuses on ECS modeling and orchestration. It does not provide a UI layer, an opinionated
agent product shell, or a full provider credential management system.
