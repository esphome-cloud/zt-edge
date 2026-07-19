use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::registry::ComponentId;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBindings {
    pub gpio_pins: Vec<u8>,
    pub sensor_ids: Vec<String>,
    pub file_paths: Vec<String>,
    pub network_hosts: Vec<String>,
    pub peer_addresses: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagerFeature {
    Heartbeat,
    Scheduler,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDefinition {
    pub id: String,
    pub role: String,
    pub components: Vec<ComponentId>,
    pub exclusive_selections: HashMap<String, ComponentId>,
    #[serde(default)]
    pub resource_bindings: ResourceBindings,
    #[serde(default)]
    pub manager_features: Vec<ManagerFeature>,
}

pub struct ProfileRegistry {
    profiles: HashMap<String, ProfileDefinition>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    pub fn register(&mut self, profile: ProfileDefinition) {
        self.profiles.insert(profile.id.clone(), profile);
    }

    pub fn get(&self, id: &str) -> Option<&ProfileDefinition> {
        self.profiles.get(id)
    }

    pub fn all_ids(&self) -> impl Iterator<Item = &str> {
        self.profiles.keys().map(String::as_str)
    }

    pub fn default_profiles() -> Self {
        Self::new()
    }
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::default_profiles()
    }
}
