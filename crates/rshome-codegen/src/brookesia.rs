use std::path::Path;

use rshome_config::ValidatedConfig;
use tera::{Context, Tera};

use crate::cmake::{generate_component_cmake, generate_root_cmake};
use crate::error::CodegenError;
use crate::generator::{ensure_dir, write_file, GeneratedProject};
use crate::sdkconfig::generate_sdkconfig;

const MAIN_TEMPLATE: &str = include_str!("../templates/brookesia/main.cpp.tera");
const COMPOSITION_HEADER: &str = include_str!("../templates/brookesia/app_composition.h");
const COMPOSITION_TEMPLATE: &str = include_str!("../templates/brookesia/app_composition.cpp.tera");
const PARTITIONS: &str = include_str!("../templates/partitions/default_16mb.csv");
const LICENSE: &str = include_str!("../templates/LICENSE");

pub struct BrookesiaGenerator<'a> {
    config: &'a ValidatedConfig,
}

impl<'a> BrookesiaGenerator<'a> {
    pub const fn new(config: &'a ValidatedConfig) -> Self {
        Self { config }
    }

    pub fn generate(&self, output_dir: &Path) -> Result<GeneratedProject, CodegenError> {
        let mut files_written = Vec::new();
        ensure_dir(output_dir)?;

        let mut context = Context::new();
        context.insert("project_name", &self.config.esphome.name);
        let main = Tera::one_off(MAIN_TEMPLATE, &context, false)?;
        let composition = Tera::one_off(COMPOSITION_TEMPLATE, &context, false)?;

        files_written.push(write_file(
            &output_dir.join("CMakeLists.txt"),
            &generate_root_cmake(&self.config.esphome.name),
        )?);
        files_written.push(write_file(
            &output_dir.join("sdkconfig.defaults"),
            &generate_sdkconfig(self.config),
        )?);
        files_written.push(write_file(&output_dir.join("partitions.csv"), PARTITIONS)?);
        files_written.push(write_file(&output_dir.join("LICENSE"), LICENSE)?);

        let main_dir = output_dir.join("main");
        ensure_dir(&main_dir)?;
        files_written.push(write_file(
            &main_dir.join("CMakeLists.txt"),
            &generate_component_cmake(&["main.cpp"], &[], &["brookesia_service_manager"]),
        )?);
        files_written.push(write_file(
            &main_dir.join("idf_component.yml"),
            "dependencies:\n  espressif/brookesia_service_manager:\n    version: \"*\"\n  espressif/brookesia_service_custom:\n    version: \"*\"\n",
        )?);
        files_written.push(write_file(&main_dir.join("main.cpp"), &main)?);

        let app_dir = output_dir.join("components/app");
        let include_dir = app_dir.join("include");
        let source_dir = app_dir.join("src");
        ensure_dir(&include_dir)?;
        ensure_dir(&source_dir)?;
        files_written.push(write_file(
            &app_dir.join("CMakeLists.txt"),
            &generate_component_cmake(
                &["src/app_composition.cpp"],
                &[],
                &["brookesia_service_manager", "brookesia_service_custom"],
            ),
        )?);
        files_written.push(write_file(
            &include_dir.join("app_composition.h"),
            COMPOSITION_HEADER,
        )?);
        files_written.push(write_file(
            &source_dir.join("app_composition.cpp"),
            &composition,
        )?);

        Ok(GeneratedProject {
            root_dir: output_dir.to_owned(),
            files_written,
            component_count: 1,
        })
    }
}
