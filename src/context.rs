use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ContextDocument;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextSource {
    Inline,
    File(String),
    Generated(String),
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ContextPayload {
    pub text: String,
}

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextEmbeddingStatus {
    NotIndexed,
    Pending,
    Indexed,
    Failed,
}

#[derive(Bundle)]
pub struct ContextBundle {
    pub document: ContextDocument,
    pub source: ContextSource,
    pub payload: ContextPayload,
    pub embedding_status: ContextEmbeddingStatus,
}

impl ContextBundle {
    pub fn new(source: ContextSource, text: impl Into<String>) -> Self {
        Self {
            document: ContextDocument,
            source,
            payload: ContextPayload { text: text.into() },
            embedding_status: ContextEmbeddingStatus::NotIndexed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMatch {
    pub entity: Entity,
    pub score: usize,
}

#[derive(Resource, Default, Clone, Debug)]
pub struct ContextIndex {
    tokens_by_entity: HashMap<Entity, HashSet<String>>,
}

impl ContextIndex {
    pub fn search_candidates(
        &self,
        candidates: impl IntoIterator<Item = Entity>,
        query: &str,
        top_k: usize,
    ) -> Vec<ContextMatch> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || top_k == 0 {
            return Vec::new();
        }

        let mut matches: Vec<ContextMatch> = candidates
            .into_iter()
            .filter_map(|entity| {
                let tokens = self.tokens_by_entity.get(&entity)?;
                let score = tokens.intersection(&query_tokens).count();
                (score > 0).then_some(ContextMatch { entity, score })
            })
            .collect();

        matches.sort_by(|left, right| right.score.cmp(&left.score));
        matches.truncate(top_k);
        matches
    }
}

pub fn spawn_context(world: &mut World, source: ContextSource, text: impl Into<String>) -> Entity {
    world.spawn(ContextBundle::new(source, text)).id()
}

pub fn rebuild_context_index(world: &mut World) {
    let mut tokens_by_entity = HashMap::new();
    let mut query = world.query::<(Entity, &ContextPayload)>();
    for (entity, payload) in query.iter(world) {
        tokens_by_entity.insert(entity, tokenize(&payload.text));
    }

    world.resource_mut::<ContextIndex>().tokens_by_entity = tokens_by_entity;
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}
