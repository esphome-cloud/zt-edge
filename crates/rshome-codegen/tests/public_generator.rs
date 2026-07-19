use rshome_codegen::ProjectGenerator;
use rshome_config::validated::{
    DependencyGraph, FrameworkType, ValidatedConfig, ValidatedEsphomeBlock,
};
use rshome_schema::ChipTarget;

fn validated_config() -> ValidatedConfig {
    ValidatedConfig {
        esphome: ValidatedEsphomeBlock {
            name: "public_fixture".to_owned(),
            chip_target: ChipTarget::Esp32,
            board: "esp32dev".to_owned(),
            friendly_name: None,
            framework_type: FrameworkType::EspIdf,
            project: None,
            solution_id: None,
            solution_variant_id: None,
        },
        components: Vec::new(),
        active_flags: Vec::new(),
        pin_allocations: Vec::new(),
        dependency_graph: DependencyGraph::new(),
    }
}

#[test]
fn default_brookesia_generator_emits_only_public_files() {
    let files = ProjectGenerator::new(&validated_config())
        .generate_in_memory()
        .expect("default Brookesia config generates a project");

    let paths: Vec<_> = files
        .keys()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "CMakeLists.txt",
            "LICENSE",
            "components/app/CMakeLists.txt",
            "components/app/include/app_composition.h",
            "components/app/src/app_composition.cpp",
            "main/CMakeLists.txt",
            "main/idf_component.yml",
            "main/main.cpp",
            "partitions.csv",
            "sdkconfig.defaults",
        ]
    );
    let generated_text = files
        .values()
        .map(|contents| String::from_utf8_lossy(contents))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(generated_text.contains("brookesia_service_manager"));
    for marker in [
        "mi".to_owned() + "mi",
        "c".to_owned() + "law" + "room",
        "bas".to_owned() + "tion",
    ] {
        assert!(!generated_text.to_ascii_lowercase().contains(&marker));
    }
}
