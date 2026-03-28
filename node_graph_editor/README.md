# Bevy Node Graph

Minimal Bevy node graph editor inspired by ComfyUI, built entirely with native `bevy_ui`.

## Run

```bash
cargo run --manifest-path node_graph_editor/Cargo.toml
```

## MVP features

- Dark pannable canvas with grid
- Draggable nodes
- Typed input/output ports with colored dots
- Click-to-connect wiring with type validation
- Sidebar palette for spawning nodes
- Seeded demo graph with ComfyUI-style node shapes
