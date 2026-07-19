use rmcp::{model::*, ErrorData as McpError};

use crate::error;
use crate::RshomeHaMcp;

// Resource URI constants
const ENTITIES_CONFIG_URI: &str = "ha://config/entities";
const SERVICES_CONFIG_URI: &str = "ha://config/services";
const RUNTIME_INFO_URI: &str = "ha://runtime/info";

impl RshomeHaMcp {
    pub fn list_resources_impl(&self) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                Resource {
                    raw: RawResource {
                        uri: ENTITIES_CONFIG_URI.to_string(),
                        name: "Entity Registry".to_string(),
                        description: Some(
                            "All registered entity IDs and their domains".to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                    },
                    annotations: None,
                },
                Resource {
                    raw: RawResource {
                        uri: SERVICES_CONFIG_URI.to_string(),
                        name: "Service Registry".to_string(),
                        description: Some(
                            "All registered services with domain and name".to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                    },
                    annotations: None,
                },
                Resource {
                    raw: RawResource {
                        uri: RUNTIME_INFO_URI.to_string(),
                        name: "Runtime Info".to_string(),
                        description: Some("Runtime configuration and statistics".to_string()),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                    },
                    annotations: None,
                },
            ],
            next_cursor: None,
        })
    }

    pub fn list_resource_templates_impl(&self) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                ResourceTemplate {
                    raw: RawResourceTemplate {
                        uri_template: "state://entities/{entity_id}".to_string(),
                        name: "Entity State".to_string(),
                        description: Some(
                            "Current state snapshot for a specific entity".to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                    },
                    annotations: None,
                },
                ResourceTemplate {
                    raw: RawResourceTemplate {
                        uri_template: "device://info/{device_id}".to_string(),
                        name: "Device Info".to_string(),
                        description: Some(
                            "Descriptor for a specific device including all entity IDs".to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                    },
                    annotations: None,
                },
            ],
            next_cursor: None,
        })
    }

    pub async fn read_resource_impl(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        match uri {
            ENTITIES_CONFIG_URI => {
                let ids = self.entity_registry.list_all();
                let entries: Vec<serde_json::Value> = ids
                    .iter()
                    .map(|id| serde_json::json!({ "entity_id": id.to_string(), "domain": id.domain() }))
                    .collect();
                let json = serde_json::to_string_pretty(&entries).map_err(error::json_error)?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(json, uri)],
                })
            }
            SERVICES_CONFIG_URI => {
                let services = self
                    .service_registry
                    .ask(rshome_svc::ServiceMsg::List)
                    .await
                    .map_err(error::actor_error)?;
                let json = serde_json::to_string_pretty(&services).map_err(error::json_error)?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(json, uri)],
                })
            }
            RUNTIME_INFO_URI => {
                let result = serde_json::json!({
                    "version": self.runtime_config.version,
                    "entity_count": self.entity_registry.count(),
                    "state_count": self.state_store.count(),
                    "native_api_running": self.native_api.is_some(),
                });
                let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(json, uri)],
                })
            }
            _ if uri.starts_with("state://entities/") => {
                let entity_id_str = uri.strip_prefix("state://entities/").unwrap_or("");
                let entity_id = rshome_entity::EntityId(entity_id_str.to_string());
                let snapshot = self.state_store.snapshot(&entity_id).ok_or_else(|| {
                    McpError::resource_not_found(format!("entity not found: {entity_id_str}"), None)
                })?;
                let last_updated = snapshot
                    .last_updated
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let result = serde_json::json!({
                    "entity_id": snapshot.entity_id.to_string(),
                    "state": snapshot.state,
                    "last_updated": last_updated,
                });
                let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(json, uri)],
                })
            }
            _ if uri.starts_with("device://info/") => {
                let device_id_str = uri.strip_prefix("device://info/").unwrap_or("");
                let dev_id = rshome_entity::DeviceId(device_id_str.to_string());
                let device_ref = self
                    .device_manager
                    .ask(|reply| rshome_entity::DeviceManagerMsg::GetDevice { id: dev_id, reply })
                    .await
                    .map_err(error::actor_error)?
                    .ok_or_else(|| {
                        McpError::resource_not_found(
                            format!("device not found: {device_id_str}"),
                            None,
                        )
                    })?;
                let info = device_ref
                    .ask(rshome_entity::DeviceMsg::GetInfo)
                    .await
                    .map_err(error::actor_error)?;
                let entity_ids = device_ref
                    .ask(rshome_entity::DeviceMsg::GetEntities)
                    .await
                    .map_err(error::actor_error)?;
                let result = serde_json::json!({
                    "device": info,
                    "entity_ids": entity_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                });
                let json = serde_json::to_string_pretty(&result).map_err(error::json_error)?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(json, uri)],
                })
            }
            _ => Err(McpError::resource_not_found(
                format!("unknown resource: {uri}"),
                None,
            )),
        }
    }
}
