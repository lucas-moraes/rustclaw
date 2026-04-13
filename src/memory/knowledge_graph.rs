use crate::error::{AgentError, MemoryError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub entity_type: EntityType,
    pub properties: HashMap<String, serde_json::Value>,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum EntityType {
    Person,
    Project,
    Concept,
    Tool,
    Skill,
    Document,
    Code,
    Location,
    Event,
    Organization,
    Other,
}

impl EntityType {
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "person" => EntityType::Person,
            "project" => EntityType::Project,
            "concept" => EntityType::Concept,
            "tool" => EntityType::Tool,
            "skill" => EntityType::Skill,
            "document" | "doc" => EntityType::Document,
            "code" | "source" => EntityType::Code,
            "location" | "place" => EntityType::Location,
            "event" => EntityType::Event,
            "organization" | "org" => EntityType::Organization,
            _ => EntityType::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Person => "person",
            EntityType::Project => "project",
            EntityType::Concept => "concept",
            EntityType::Tool => "tool",
            EntityType::Skill => "skill",
            EntityType::Document => "document",
            EntityType::Code => "code",
            EntityType::Location => "location",
            EntityType::Event => "event",
            EntityType::Organization => "organization",
            EntityType::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
    pub properties: HashMap<String, serde_json::Value>,
    pub weight: f32,
    pub created_at: std::time::SystemTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RelationType {
    Uses,
    DependsOn,
    Implements,
    Extends,
    Contains,
    References,
    ParticipatedIn,
    CreatedBy,
    PartOf,
    RelatedTo,
    Follows,
    Leads,
    CollaboratesWith,
    Other,
}

impl RelationType {
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "uses" | "use" => RelationType::Uses,
            "depends_on" | "depend" | "depends" => RelationType::DependsOn,
            "implements" | "implement" => RelationType::Implements,
            "extends" | "extend" => RelationType::Extends,
            "contains" | "include" => RelationType::Contains,
            "references" | "reference" | "ref" => RelationType::References,
            "participated_in" | "participate" => RelationType::ParticipatedIn,
            "created_by" | "create" => RelationType::CreatedBy,
            "part_of" | "part" => RelationType::PartOf,
            "related_to" | "related" => RelationType::RelatedTo,
            "follows" | "follow" => RelationType::Follows,
            "leads" | "lead" => RelationType::Leads,
            "collaborates_with" | "collaborate" => RelationType::CollaboratesWith,
            _ => RelationType::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RelationType::Uses => "uses",
            RelationType::DependsOn => "depends_on",
            RelationType::Implements => "implements",
            RelationType::Extends => "extends",
            RelationType::Contains => "contains",
            RelationType::References => "references",
            RelationType::ParticipatedIn => "participated_in",
            RelationType::CreatedBy => "created_by",
            RelationType::PartOf => "part_of",
            RelationType::RelatedTo => "related_to",
            RelationType::Follows => "follows",
            RelationType::Leads => "leads",
            RelationType::CollaboratesWith => "collaborates_with",
            RelationType::Other => "related_to",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    entities: HashMap<String, Entity>,
    relationships: HashMap<String, Relationship>,
    entity_index: HashMap<String, HashSet<String>>,
    relation_index: HashMap<String, HashSet<String>>,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            relationships: HashMap::new(),
            entity_index: HashMap::new(),
            relation_index: HashMap::new(),
        }
    }

    pub fn add_entity(&mut self, entity: Entity) -> Result<(), AgentError> {
        let id = entity.id.clone();
        
        if self.entities.contains_key(&id) {
            return Err(AgentError::Memory(MemoryError::NotFound(format!(
                "Entity {} already exists",
                id
            ))));
        }
        
        self.entities.insert(id.clone(), entity);
        
        let name_lower = id.to_lowercase();
        self.entity_index
            .entry(name_lower)
            .or_insert_with(HashSet::new)
            .insert(id);
        
        Ok(())
    }

    pub fn get_entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn get_entity_mut(&mut self, id: &str) -> Option<&mut Entity> {
        self.entities.get_mut(id)
    }

    pub fn update_entity(&mut self, id: &str, updates: HashMap<String, serde_json::Value>) -> Result<(), AgentError> {
        if let Some(entity) = self.entities.get_mut(id) {
            for (key, value) in updates {
                entity.properties.insert(key, value);
            }
            entity.updated_at = std::time::SystemTime::now();
            Ok(())
        } else {
            Err(AgentError::Memory(MemoryError::NotFound(format!(
                "Entity {} not found",
                id
            ))))
        }
    }

    pub fn delete_entity(&mut self, id: &str) -> Result<(), AgentError> {
        if !self.entities.contains_key(id) {
            return Err(AgentError::Memory(MemoryError::NotFound(format!(
                "Entity {} not found",
                id
            ))));
        }
        
        let related_relations: Vec<String> = self.relationships
            .values()
            .filter(|r| r.source_id == id || r.target_id == id)
            .map(|r| r.id.clone())
            .collect();
        
        for rel_id in related_relations {
            self.relationships.remove(&rel_id);
        }
        
        self.entities.remove(id);
        
        let name_lower = id.to_lowercase();
        if let Some(ids) = self.entity_index.get_mut(&name_lower) {
            ids.remove(id);
        }
        
        Ok(())
    }

    pub fn add_relationship(&mut self, relationship: Relationship) -> Result<(), AgentError> {
        let id = relationship.id.clone();
        let source_id = relationship.source_id.clone();
        let target_id = relationship.target_id.clone();
        
        if !self.entities.contains_key(&source_id) {
            return Err(AgentError::Memory(MemoryError::NotFound(format!(
                "Source entity {} not found",
                source_id
            ))));
        }
        
        if !self.entities.contains_key(&target_id) {
            return Err(AgentError::Memory(MemoryError::NotFound(format!(
                "Target entity {} not found",
                target_id
            ))));
        }
        
        self.relationships.insert(id.clone(), relationship);
        
        self.relation_index
            .entry(source_id.clone())
            .or_insert_with(HashSet::new)
            .insert(id.clone());
        
        self.relation_index
            .entry(target_id.clone())
            .or_insert_with(HashSet::new)
            .insert(id);
        
        Ok(())
    }

    pub fn get_relationships_from(&self, entity_id: &str) -> Vec<&Relationship> {
        self.relation_index
            .get(entity_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.relationships.get(id))
                    .filter(|r| r.source_id == entity_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_relationships_to(&self, entity_id: &str) -> Vec<&Relationship> {
        self.relation_index
            .get(entity_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.relationships.get(id))
                    .filter(|r| r.target_id == entity_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_connected_entities(&self, entity_id: &str) -> Vec<(&Entity, &Relationship)> {
        let mut results = Vec::new();
        
        for rel in self.get_relationships_from(entity_id) {
            if let Some(target) = self.entities.get(&rel.target_id) {
                results.push((target, rel));
            }
        }
        
        for rel in self.get_relationships_to(entity_id) {
            if let Some(source) = self.entities.get(&rel.source_id) {
                results.push((source, rel));
            }
        }
        
        results
    }

    pub fn find_entities(&self, query: &str) -> Vec<&Entity> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        
        for entity in self.entities.values() {
            if entity.name.to_lowercase().contains(&query_lower) {
                results.push(entity);
            }
        }
        
        results
    }

    pub fn find_entities_by_type(&self, entity_type: EntityType) -> Vec<&Entity> {
        self.entities
            .values()
            .filter(|e| e.entity_type == entity_type)
            .collect()
    }

    pub fn find_path(&self, source_id: &str, target_id: &str, max_depth: usize) -> Option<Vec<Relationship>> {
        if source_id == target_id {
            return Some(vec![]);
        }
        
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: Vec<(String, Vec<Relationship>)> = vec![(source_id.to_string(), vec![])];
        
        while let Some((current, path)) = queue.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());
            
            for rel in self.get_relationships_from(&current) {
                let mut new_path = path.clone();
                new_path.push(rel.clone());
                
                if rel.target_id == target_id {
                    return Some(new_path);
                }
                
                if new_path.len() < max_depth {
                    queue.push((rel.target_id.clone(), new_path));
                }
            }
        }
        
        None
    }

    pub fn get_subgraph(&self, entity_ids: &[String]) -> KnowledgeGraph {
        let mut subgraph = KnowledgeGraph::new();
        
        for id in entity_ids {
            if let Some(entity) = self.entities.get(id) {
                subgraph.entities.insert(entity.id.clone(), entity.clone());
            }
        }
        
        for rel in self.relationships.values() {
            if subgraph.entities.contains_key(&rel.source_id) && subgraph.entities.contains_key(&rel.target_id) {
                subgraph.relationships.insert(rel.id.clone(), rel.clone());
            }
        }
        
        subgraph
    }

    pub fn get_stats(&self) -> GraphStats {
        let mut type_counts: HashMap<String, usize> = HashMap::new();
        for entity in self.entities.values() {
            *type_counts.entry(entity.entity_type.as_str().to_string()).or_insert(0) += 1;
        }
        
        let mut relation_counts: HashMap<String, usize> = HashMap::new();
        for rel in self.relationships.values() {
            *relation_counts.entry(rel.relation_type.as_str().to_string()).or_insert(0) += 1;
        }
        
        GraphStats {
            total_entities: self.entities.len(),
            total_relationships: self.relationships.len(),
            entity_type_counts: type_counts,
            relation_type_counts: relation_counts,
        }
    }

    pub fn entities(&self) -> &HashMap<String, Entity> {
        &self.entities
    }

    pub fn relationships(&self) -> &HashMap<String, Relationship> {
        &self.relationships
    }

    pub fn import_from_memory(&mut self, memories: &[crate::memory::MemoryEntry]) -> Result<usize, AgentError> {
        let mut count = 0;
        
        for memory in memories {
            let entity_id = format!("memory_{}", memory.id);
            
            let entity = Entity {
                id: entity_id.clone(),
                name: memory.content.chars().take(50).collect::<String>(),
                entity_type: EntityType::Concept,
                properties: {
                    let mut props = HashMap::new();
                    props.insert("memory_id".to_string(), serde_json::json!(memory.id));
                    props.insert("memory_type".to_string(), serde_json::json!(format!("{:?}", memory.memory_type)));
                    props
                },
                created_at: std::time::SystemTime::now(),
                updated_at: std::time::SystemTime::now(),
            };
            
            if self.add_entity(entity).is_ok() {
                count += 1;
            }
        }
        
        Ok(count)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_entities: usize,
    pub total_relationships: usize,
    pub entity_type_counts: HashMap<String, usize>,
    pub relation_type_counts: HashMap<String, usize>,
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KnowledgeGraphStore {
    graph: Arc<RwLock<KnowledgeGraph>>,
}

impl KnowledgeGraphStore {
    pub fn new() -> Self {
        Self {
            graph: Arc::new(RwLock::new(KnowledgeGraph::new())),
        }
    }

    pub async fn add_entity(&self, entity: Entity) -> Result<(), AgentError> {
        self.graph.write().await.add_entity(entity)
    }

    pub async fn get_entity(&self, id: &str) -> Option<Entity> {
        self.graph.read().await.get_entity(id).cloned()
    }

    pub async fn add_relationship(&self, relationship: Relationship) -> Result<(), AgentError> {
        self.graph.write().await.add_relationship(relationship)
    }

    pub async fn find_entities(&self, query: &str) -> Vec<Entity> {
        self.graph.read().await.find_entities(query).into_iter().cloned().collect()
    }

    pub async fn get_connected_entities(&self, entity_id: &str) -> Vec<(Entity, Relationship)> {
        self.graph.read().await.get_connected_entities(entity_id)
            .into_iter()
            .map(|(e, r)| (e.clone(), r.clone()))
            .collect()
    }

    pub async fn get_stats(&self) -> GraphStats {
        self.graph.read().await.get_stats()
    }

    pub async fn import_from_memory(&self, memories: &[crate::memory::MemoryEntry]) -> Result<usize, AgentError> {
        self.graph.write().await.import_from_memory(memories)
    }

    pub fn into_inner(self) -> Arc<RwLock<KnowledgeGraph>> {
        self.graph
    }
}

impl Default for KnowledgeGraphStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entity(name: &str, entity_type: EntityType) -> Entity {
        Entity {
            id: name.to_string(),
            name: name.to_string(),
            entity_type,
            properties: HashMap::new(),
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
        }
    }

    #[test]
    fn test_add_and_get_entity() {
        let mut graph = KnowledgeGraph::new();
        let entity = create_test_entity("test", EntityType::Project);
        
        graph.add_entity(entity.clone()).unwrap();
        assert_eq!(graph.get_entity("test"), Some(&entity));
    }

    #[test]
    fn test_add_relationship() {
        let mut graph = KnowledgeGraph::new();
        
        let entity1 = create_test_entity("agent", EntityType::Person);
        let entity2 = create_test_entity("tool", EntityType::Tool);
        
        graph.add_entity(entity1).unwrap();
        graph.add_entity(entity2).unwrap();
        
        let relationship = Relationship {
            id: "uses".to_string(),
            source_id: "agent".to_string(),
            target_id: "tool".to_string(),
            relation_type: RelationType::Uses,
            properties: HashMap::new(),
            weight: 1.0,
            created_at: std::time::SystemTime::now(),
        };
        
        graph.add_relationship(relationship).unwrap();
        
        let connected = graph.get_connected_entities("agent");
        assert_eq!(connected.len(), 1);
    }

    #[test]
    fn test_find_path() {
        let mut graph = KnowledgeGraph::new();
        
        graph.add_entity(create_test_entity("A", EntityType::Concept)).unwrap();
        graph.add_entity(create_test_entity("B", EntityType::Concept)).unwrap();
        graph.add_entity(create_test_entity("C", EntityType::Concept)).unwrap();
        
        graph.add_relationship(Relationship {
            id: "r1".to_string(),
            source_id: "A".to_string(),
            target_id: "B".to_string(),
            relation_type: RelationType::RelatedTo,
            properties: HashMap::new(),
            weight: 1.0,
            created_at: std::time::SystemTime::now(),
        }).unwrap();
        
        graph.add_relationship(Relationship {
            id: "r2".to_string(),
            source_id: "B".to_string(),
            target_id: "C".to_string(),
            relation_type: RelationType::RelatedTo,
            properties: HashMap::new(),
            weight: 1.0,
            created_at: std::time::SystemTime::now(),
        }).unwrap();
        
        let path = graph.find_path("A", "C", 10);
        assert!(path.is_some());
        assert_eq!(path.unwrap().len(), 2);
    }

    #[test]
    fn test_get_stats() {
        let mut graph = KnowledgeGraph::new();
        graph.add_entity(create_test_entity("p1", EntityType::Project)).unwrap();
        graph.add_entity(create_test_entity("p2", EntityType::Project)).unwrap();
        graph.add_entity(create_test_entity("c1", EntityType::Concept)).unwrap();
        
        let stats = graph.get_stats();
        assert_eq!(stats.total_entities, 3);
        assert_eq!(stats.entity_type_counts.get("project"), Some(&2));
        assert_eq!(stats.entity_type_counts.get("concept"), Some(&1));
    }
}