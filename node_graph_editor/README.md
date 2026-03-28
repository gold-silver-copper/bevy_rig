# Bevy Rig Graph

Minimal Bevy-native node graph editor for composing Rig-style agents from individual field nodes, built entirely with `bevy_ui`.

## Run

```bash
cargo run --manifest-path node_graph_editor/Cargo.toml
```

## MVP features

- Dark pannable canvas with grid
- Draggable typed nodes and click-to-connect wiring
- One node type per agent field: `name`, `description`, `model`, `preamble`, `static_context`, `temperature`, `max_tokens`, `additional_params`, `tool_server_handle`, `dynamic_context`, `tool_choice`, `default_max_turns`, `hook`, `output_schema`
- Dedicated `Prompt` and `Text Output` nodes
- Local Ollama model discovery inside the app
- Run the selected `Agent` node by compiling its connected field graph into an ephemeral Rig agent
- Response delivery into the connected `Text Output` node

## Current runtime scope

- Executable now: model, prompt, name, description, preamble, static context, temperature, max tokens, additional params, default max turns, output schema
- Stored but not executed yet: tool server handle, dynamic context, hook
- Provider support today: local Ollama
