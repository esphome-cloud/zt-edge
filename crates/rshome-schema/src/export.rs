//! Schema export: JSON Schema generation for the component registry.
//!
//! The primary export is `ComponentRegistry::to_json_schema()` which leverages
//! the `#[derive(JsonSchema)]` attributes on all entity types to produce a
//! machine-readable JSON Schema document.  This can be consumed by editors,
//! wizards, and validation tools.
//!
//! GraphQL export is feature-gated to the `graphql` feature and is deferred
//! to Phase 6.

use schemars::{schema_for, Schema};

use crate::entity::EntitySchema;
use crate::registry::{ComponentDefinition, ComponentRegistry};

impl ComponentRegistry {
    /// Generate a JSON Schema for the full `EntitySchema` union.
    ///
    /// The resulting schema describes all valid entity configurations that can
    /// appear in a YAE/rshome device config.  Use `serde_json::to_string` on
    /// the returned value to get the schema as a JSON string.
    pub fn to_json_schema(&self) -> Schema {
        schema_for!(EntitySchema)
    }

    /// Serialize `to_json_schema()` directly to a JSON string.
    ///
    /// Returns the serialized schema or a JSON error object on failure.
    pub fn to_json_schema_string(&self) -> String {
        let schema = self.to_json_schema();
        serde_json::to_string_pretty(&schema).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Return all component definitions as a sorted list (for MCP / REST APIs).
    pub fn to_component_list(&self) -> Vec<&ComponentDefinition> {
        let mut list: Vec<&ComponentDefinition> = self.all_definitions().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }
}

// ── GraphQL schema (Phase 6) ───────────────────────────────────────────────────

/// GraphQL schema for the rshome component registry and config validation.
///
/// Enable with `--features graphql`.  Build the schema with:
///
/// ```rust,ignore
/// use rshome_schema::export::graphql::schema;
/// let s = schema();
/// ```
///
/// Introspect with any standard GraphQL tool (e.g. Altair, GraphiQL).
#[cfg(feature = "graphql")]
pub mod graphql {
    use juniper::{graphql_object, EmptySubscription, FieldResult, GraphQLObject, RootNode};

    use crate::feature_flags::FeatureFlagSet;
    use crate::registry::{ComponentId, ComponentRegistry};

    // ── Context ───────────────────────────────────────────────────────────────

    /// GraphQL execution context carrying the component registry.
    pub struct SchemaContext {
        registry: ComponentRegistry,
    }

    impl juniper::Context for SchemaContext {}

    impl SchemaContext {
        pub fn new() -> Self {
            Self {
                registry: ComponentRegistry::default_registry(),
            }
        }
    }

    impl Default for SchemaContext {
        fn default() -> Self {
            Self::new()
        }
    }

    // ── Output types ──────────────────────────────────────────────────────────

    /// A registered component definition.
    #[derive(GraphQLObject)]
    #[graphql(description = "A component or platform definition in the rshome registry")]
    pub struct GqlComponent {
        /// Unique component identifier (e.g. "dht", "wifi").
        pub id: String,
        /// Human-readable summary for docs and component pickers.
        pub description: String,
        /// Whether this is an abstract family parent (e.g. "sensor").
        pub is_family: bool,
        /// Entity type produced by this component, if any.
        pub entity_type: Option<String>,
        /// Component IDs auto-loaded when this component is selected.
        pub auto_load: Vec<String>,
        /// Component IDs that must be present for this component to work.
        pub dependencies: Vec<String>,
        /// Component IDs that cannot coexist with this component.
        pub conflicts_with: Vec<String>,
        /// Child component implementations (non-empty only for family parents).
        pub child_components: Vec<String>,
    }

    impl From<&crate::registry::ComponentDefinition> for GqlComponent {
        fn from(def: &crate::registry::ComponentDefinition) -> Self {
            Self {
                id: def.id.clone(),
                description: def.description.clone(),
                is_family: def.is_family,
                entity_type: def.entity_type.map(entity_type_name),
                auto_load: def.auto_load.clone(),
                dependencies: def.dependencies.clone(),
                conflicts_with: def.conflicts_with.clone(),
                child_components: def.child_components.clone(),
            }
        }
    }

    fn entity_type_name(entity_type: crate::EntityType) -> String {
        serde_json::to_string(&entity_type)
            .expect("entity type serializes")
            .trim_matches('"')
            .to_string()
    }

    /// A GPIO pin descriptor.
    #[derive(GraphQLObject)]
    #[graphql(description = "GPIO pin capability information for a chip target")]
    pub struct GqlPin {
        /// GPIO number (0-based).
        pub gpio_num: i32,
        /// True if the pin is input-only.
        pub input_only: bool,
        /// True if the pin is reserved for internal flash.
        pub flash_reserved: bool,
        /// True if the pin affects boot mode when driven at startup.
        pub is_strapping: bool,
        /// Human-readable description.
        pub description: String,
    }

    /// Feature flag computation result.
    #[derive(GraphQLObject)]
    #[graphql(description = "Computed USE_* feature flags for a component selection")]
    pub struct GqlFeatureFlags {
        /// List of active USE_* flag names.
        pub flags: Vec<String>,
        /// Generated C preprocessor defines block.
        pub c_defines: String,
        /// Cargo feature names for conditional compilation.
        pub cargo_features: Vec<String>,
    }

    /// Result of adding a component to a config.
    #[derive(GraphQLObject)]
    pub struct GqlAddComponentResult {
        /// Whether the operation succeeded.
        pub ok: bool,
        /// Updated config JSON (canonical), or empty string on failure.
        pub config_json: String,
        /// Auto-loaded component IDs that were added.
        pub auto_loaded: Vec<String>,
        /// Error message if ok=false.
        pub error: Option<String>,
    }

    /// Result of removing a component from a config.
    #[derive(GraphQLObject)]
    pub struct GqlRemoveComponentResult {
        /// Whether the operation succeeded.
        pub ok: bool,
        /// Updated config JSON (canonical), or empty string on failure.
        pub config_json: String,
        /// Error message if ok=false.
        pub error: Option<String>,
    }

    /// Dependency tree for a component.
    #[derive(GraphQLObject)]
    pub struct GqlDependencyTree {
        /// Root component ID.
        pub root: String,
        /// All transitive auto-load IDs (sorted).
        pub auto_loaded: Vec<String>,
        /// Declared dependency IDs.
        pub dependencies: Vec<String>,
        /// Conflicts with these IDs.
        pub conflicts_with: Vec<String>,
    }

    // ── Query ─────────────────────────────────────────────────────────────────

    pub struct Query;

    #[graphql_object]
    #[graphql(context = SchemaContext)]
    impl Query {
        /// List all registered components, optionally filtered by entity type category.
        fn components(
            context: &SchemaContext,
            #[graphql(description = "Filter by entity type (e.g. \"sensor\", \"switch\")")]
            category: Option<String>,
        ) -> FieldResult<Vec<GqlComponent>> {
            let mut comps: Vec<GqlComponent> = context
                .registry
                .all_definitions()
                .map(GqlComponent::from)
                .collect();

            if let Some(ref cat) = category {
                let cat_lower = cat.to_lowercase();
                comps.retain(|c| {
                    c.entity_type
                        .as_deref()
                        .map(|e| e == cat_lower)
                        .unwrap_or(false)
                });
            }

            comps.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(comps)
        }

        /// Get a single component definition by ID.
        fn component(
            context: &SchemaContext,
            #[graphql(description = "Component ID, e.g. \"dht\", \"wifi\"")] id: String,
        ) -> FieldResult<Option<GqlComponent>> {
            Ok(context.registry.get(&id).map(GqlComponent::from))
        }

        /// Get GPIO pin map for a chip target.
        fn pin_map(
            #[graphql(description = "Chip target name, e.g. \"esp32\", \"esp32s3\", \"esp32c6\"")]
            target: String,
        ) -> FieldResult<Vec<GqlPin>> {
            use crate::platform::ChipTarget;

            let chip = match target.to_lowercase().as_str() {
                "esp32" => ChipTarget::Esp32,
                "esp32s2" | "esp32_s2" | "esp32-s2" => ChipTarget::Esp32S2,
                "esp32s3" | "esp32_s3" | "esp32-s3" => ChipTarget::Esp32S3,
                "esp32c2" | "esp32_c2" | "esp32-c2" => ChipTarget::Esp32C2,
                "esp32c3" | "esp32_c3" | "esp32-c3" => ChipTarget::Esp32C3,
                "esp32c5" | "esp32_c5" | "esp32-c5" => ChipTarget::Esp32C5,
                "esp32c6" | "esp32_c6" | "esp32-c6" => ChipTarget::Esp32C6,
                "esp32c61" | "esp32_c61" | "esp32-c61" => ChipTarget::Esp32C61,
                "esp32h2" | "esp32_h2" | "esp32-h2" => ChipTarget::Esp32H2,
                "esp32p4" | "esp32_p4" | "esp32-p4" => ChipTarget::Esp32P4,
                other => {
                    return Err(format!(
                        "unknown target '{other}'; supported: esp32, esp32s2, esp32s3, \
                         esp32c2, esp32c3, esp32c5, esp32c6, esp32c61, esp32h2, esp32p4"
                    )
                    .into());
                }
            };

            let caps = crate::pin::capabilities_for(chip);
            let max_gpio = caps.max_gpio;
            let input_only = caps.input_only;
            let flash_reserved = caps.flash_reserved;
            let strapping = caps.strapping;

            let pins = (0..=max_gpio)
                .map(|gpio| {
                    let inp_only = input_only.contains(&gpio);
                    let flash = flash_reserved.contains(&gpio);
                    let strap = strapping.contains(&gpio);
                    let mut notes = Vec::<&str>::new();
                    if flash {
                        notes.push("flash-reserved");
                    }
                    if strap {
                        notes.push("strapping");
                    }
                    if inp_only {
                        notes.push("input-only");
                    }
                    let description = if notes.is_empty() {
                        format!("GPIO {gpio}")
                    } else {
                        format!("GPIO {gpio} ({})", notes.join(", "))
                    };
                    GqlPin {
                        gpio_num: gpio as i32,
                        input_only: inp_only,
                        flash_reserved: flash,
                        is_strapping: strap,
                        description,
                    }
                })
                .collect();

            Ok(pins)
        }

        /// Compute USE_* feature flags for a JSON array of component IDs.
        fn feature_flags(
            context: &SchemaContext,
            #[graphql(description = "JSON array of component ID strings")] ids_json: String,
        ) -> FieldResult<GqlFeatureFlags> {
            let ids: Vec<ComponentId> =
                serde_json::from_str(&ids_json).map_err(|e| format!("invalid JSON: {e}"))?;

            let ffs = FeatureFlagSet::from_components(&ids, &context.registry);
            let mut flags: Vec<String> = ffs.iter_flags().map(|s| s.to_owned()).collect();
            flags.sort();

            Ok(GqlFeatureFlags {
                flags,
                c_defines: ffs.to_c_defines(),
                cargo_features: ffs.to_cargo_features(),
            })
        }

        /// Get the full dependency tree for a component.
        fn dependency_tree(
            context: &SchemaContext,
            #[graphql(description = "Component ID to analyze")] component_id: String,
        ) -> FieldResult<GqlDependencyTree> {
            let def = context
                .registry
                .get(&component_id)
                .ok_or_else(|| format!("component '{component_id}' not found"))?;

            let auto_loaded = context.registry.resolve_auto_load(&[component_id.clone()]);

            Ok(GqlDependencyTree {
                root: component_id,
                auto_loaded,
                dependencies: def.dependencies.clone(),
                conflicts_with: def.conflicts_with.clone(),
            })
        }
    }

    // ── Mutation ──────────────────────────────────────────────────────────────

    pub struct Mutation;

    #[graphql_object]
    #[graphql(context = SchemaContext)]
    impl Mutation {
        /// Add a component to a config JSON, resolving its dependencies.
        ///
        /// Returns the updated config JSON and the list of auto-loaded components.
        fn add_component(
            context: &SchemaContext,
            #[graphql(description = "JSON-encoded RawConfig")] config_json: String,
            #[graphql(description = "Component ID to add")] component_id: String,
            #[graphql(description = "Component config parameters as JSON object (optional)")]
            params: Option<String>,
        ) -> FieldResult<GqlAddComponentResult> {
            let mut config: serde_json::Value = serde_json::from_str(&config_json)
                .map_err(|e| format!("invalid config JSON: {e}"))?;

            // Verify the component exists.
            let def = match context.registry.get(&component_id) {
                Some(d) => d,
                None => {
                    return Ok(GqlAddComponentResult {
                        ok: false,
                        config_json: String::new(),
                        auto_loaded: vec![],
                        error: Some(format!("component '{component_id}' not found")),
                    });
                }
            };

            // Build component config value.
            let comp_config = params
                .as_deref()
                .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            // Determine auto-loaded components.
            let auto_loaded = context.registry.resolve_auto_load(&[component_id.clone()]);
            let directly_added: Vec<String> = auto_loaded
                .iter()
                .filter(|id| id.as_str() != component_id)
                .cloned()
                .collect();

            // Append the new component to the config.
            let new_entry = serde_json::json!({
                "component_type": component_id,
                "platform": def.entity_type.as_ref().map(|e| format!("{e:?}").to_lowercase()),
                "config": comp_config,
            });

            if let Some(components) = config.get_mut("components").and_then(|v| v.as_array_mut()) {
                components.push(new_entry);
            } else {
                config["components"] = serde_json::json!([new_entry]);
            }

            let updated_json = serde_json::to_string_pretty(&config)
                .map_err(|e| format!("serialization error: {e}"))?;

            Ok(GqlAddComponentResult {
                ok: true,
                config_json: updated_json,
                auto_loaded: directly_added,
                error: None,
            })
        }

        /// Remove a component from a config JSON, checking for dependents.
        fn remove_component(
            context: &SchemaContext,
            #[graphql(description = "JSON-encoded RawConfig")] config_json: String,
            #[graphql(description = "Component ID to remove")] component_id: String,
        ) -> FieldResult<GqlRemoveComponentResult> {
            let mut config: serde_json::Value = serde_json::from_str(&config_json)
                .map_err(|e| format!("invalid config JSON: {e}"))?;

            // Check for dependents within the current config (not the whole registry).
            let config_component_ids: Vec<String> = config
                .get("components")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            c.get("component_type")
                                .and_then(|t| t.as_str())
                                .map(String::from)
                        })
                        .filter(|id| id != &component_id)
                        .collect()
                })
                .unwrap_or_default();

            let dependents: Vec<String> = config_component_ids
                .iter()
                .filter(|id| {
                    context
                        .registry
                        .get(id.as_str())
                        .map(|d| d.dependencies.contains(&component_id))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();

            if !dependents.is_empty() {
                return Ok(GqlRemoveComponentResult {
                    ok: false,
                    config_json: String::new(),
                    error: Some(format!(
                        "cannot remove '{component_id}': required by {}",
                        dependents.join(", ")
                    )),
                });
            }

            // Filter out the component.
            if let Some(components) = config.get_mut("components").and_then(|v| v.as_array_mut()) {
                components.retain(|c| {
                    c.get("component_type").and_then(|t| t.as_str()) != Some(&component_id)
                });
            }

            let updated_json = serde_json::to_string_pretty(&config)
                .map_err(|e| format!("serialization error: {e}"))?;

            Ok(GqlRemoveComponentResult {
                ok: true,
                config_json: updated_json,
                error: None,
            })
        }
    }

    // ── Schema factory ────────────────────────────────────────────────────────

    /// The root schema type.
    pub type Schema = RootNode<'static, Query, Mutation, EmptySubscription<SchemaContext>>;

    /// Create the rshome GraphQL schema.
    pub fn schema() -> Schema {
        Schema::new(Query, Mutation, EmptySubscription::new())
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[cfg(test)]
    mod tests {
        use juniper::Variables;

        use super::*;

        fn ctx() -> SchemaContext {
            SchemaContext::new()
        }

        fn exec(query: &str) -> serde_json::Value {
            let schema = schema();
            let (data, errors) =
                juniper::execute_sync(query, None, &schema, &Variables::new(), &ctx())
                    .expect("GraphQL execution failed");
            assert!(errors.is_empty(), "GraphQL errors: {errors:?}");
            serde_json::to_value(data).unwrap()
        }

        #[test]
        fn query_components_returns_list() {
            let result = exec("{ components { id isPlatform } }");
            let comps = result["components"].as_array().unwrap();
            assert!(!comps.is_empty());
        }

        #[test]
        fn query_component_by_id() {
            let result = exec(r#"{ component(id: "wifi") { id isPlatform conflictsWith } }"#);
            let comp = &result["component"];
            assert_eq!(comp["id"].as_str(), Some("wifi"));
            assert!(!comp["isPlatform"].as_bool().unwrap());
        }

        #[test]
        fn query_component_missing_returns_null() {
            let result = exec(r#"{ component(id: "does_not_exist_xyz") { id } }"#);
            assert!(result["component"].is_null());
        }

        #[test]
        fn query_pin_map_esp32() {
            let result = exec(r#"{ pinMap(target: "esp32") { gpioNum inputOnly flashReserved } }"#);
            let pins = result["pinMap"].as_array().unwrap();
            assert_eq!(pins.len(), 40);
        }

        #[test]
        fn query_feature_flags() {
            let result =
                exec(r#"{ featureFlags(idsJson: "[\"wifi\",\"logger\"]") { flags cDefines } }"#);
            let flags = result["featureFlags"]["flags"].as_array().unwrap();
            assert!(flags.iter().any(|f| f.as_str() == Some("USE_WIFI")));
        }

        #[test]
        fn query_dependency_tree_dht() {
            let result = exec(r#"{ dependencyTree(componentId: "dht") { root autoLoaded } }"#);
            let tree = &result["dependencyTree"];
            assert_eq!(tree["root"].as_str(), Some("dht"));
            let auto = tree["autoLoaded"].as_array().unwrap();
            assert!(auto.iter().any(|a| a.as_str() == Some("sensor")));
        }

        #[test]
        fn mutation_add_component() {
            let config = serde_json::json!({
                "esphome": {"name": "test", "platform": "esp32", "board": "esp32dev"},
                "components": []
            });
            let config_json = serde_json::to_string(&config).unwrap();
            let query = format!(
                r#"mutation {{ addComponent(configJson: {config_json:?}, componentId: "wifi") {{ ok autoLoaded }} }}"#
            );
            let result = exec(&query);
            assert!(result["addComponent"]["ok"].as_bool().unwrap());
        }

        #[test]
        fn mutation_remove_component() {
            let config = serde_json::json!({
                "esphome": {"name": "test", "platform": "esp32", "board": "esp32dev"},
                "components": [
                    {"component_type": "wifi", "platform": null, "config": {}}
                ]
            });
            let config_json = serde_json::to_string(&config).unwrap();
            let query = format!(
                r#"mutation {{ removeComponent(configJson: {config_json:?}, componentId: "wifi") {{ ok configJson }} }}"#
            );
            let result = exec(&query);
            assert!(result["removeComponent"]["ok"].as_bool().unwrap());
        }

        #[test]
        fn query_components_filtered_by_category() {
            let result = exec(r#"{ components(category: "sensor") { id entityType } }"#);
            let comps = result["components"].as_array().unwrap();
            assert!(!comps.is_empty());
            for c in comps {
                assert_eq!(c["entityType"].as_str(), Some("sensor"));
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_json_schema_serializes_to_json() {
        let reg = ComponentRegistry::default_registry();
        let schema = reg.to_json_schema();
        let json = serde_json::to_string(&schema).unwrap();
        // Must be non-empty valid JSON containing some entity type names.
        assert!(!json.is_empty());
        assert!(json.starts_with('{') || json.starts_with('['));
    }

    #[test]
    fn to_json_schema_mentions_sensor_variant() {
        let reg = ComponentRegistry::default_registry();
        let json = reg.to_json_schema_string();
        assert!(
            json.contains("sensor"),
            "schema should reference the sensor entity type"
        );
    }

    #[test]
    fn to_json_schema_mentions_all_16_entity_types() {
        let reg = ComponentRegistry::default_registry();
        let json = reg.to_json_schema_string();
        let expected = [
            "sensor",
            "binary_sensor",
            "switch",
            "number",
            "select",
            "text",
            "button",
            "event",
            "light",
            "climate",
            "fan",
            "cover",
            "lock",
            "media_player",
            "alarm_control_panel",
            "text_sensor",
        ];
        for entity_type in &expected {
            assert!(
                json.contains(entity_type),
                "schema missing entity type '{entity_type}'"
            );
        }
    }

    #[test]
    fn to_component_list_is_sorted() {
        let reg = ComponentRegistry::default_registry();
        let list = reg.to_component_list();
        let ids: Vec<&str> = list.iter().map(|d| d.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "to_component_list should be sorted by id");
    }

    #[test]
    fn to_component_list_contains_all_registered() {
        let reg = ComponentRegistry::default_registry();
        let list = reg.to_component_list();
        assert!(
            list.len() >= 30,
            "expected ≥30 components in list, got {}",
            list.len()
        );
    }

    #[test]
    fn to_json_schema_string_is_valid_json() {
        let reg = ComponentRegistry::default_registry();
        let s = reg.to_json_schema_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("to_json_schema_string should produce valid JSON");
        assert!(parsed.is_object(), "schema root should be a JSON object");
    }
}
