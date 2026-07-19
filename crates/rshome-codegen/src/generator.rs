use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rshome_config::ValidatedConfig;

use crate::CodegenError;

thread_local! {
    static MEMORY_OUTPUT: RefCell<Option<BTreeMap<PathBuf, Vec<u8>>>> = const { RefCell::new(None) };
}

pub(crate) fn ensure_dir(path: &Path) -> Result<(), CodegenError> {
    if path.as_os_str().is_empty() || MEMORY_OUTPUT.with(|output| output.borrow().is_some()) {
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|source| CodegenError::Io {
        path: path.to_owned(),
        source,
    })
}

pub(crate) fn write_file(path: &Path, contents: &str) -> Result<PathBuf, CodegenError> {
    let in_memory = MEMORY_OUTPUT.with(|output| {
        let mut output = output.borrow_mut();
        output.as_mut().map(|files| {
            files.insert(path.to_owned(), contents.as_bytes().to_vec());
        })
    });
    if in_memory.is_some() {
        return Ok(path.to_owned());
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(path, contents).map_err(|source| CodegenError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(path.to_owned())
}

/// The result of generating a public Brookesia project.
#[derive(Debug)]
pub struct GeneratedProject {
    /// Root directory containing the generated project.
    pub root_dir: PathBuf,
    /// Every written file in deterministic path order.
    pub files_written: Vec<PathBuf>,
    /// Number of generated public components.
    pub component_count: usize,
}

/// Generates a public Brookesia ESP-IDF project from validated rshome input.
pub struct ProjectGenerator<'a> {
    config: &'a ValidatedConfig,
}

impl<'a> ProjectGenerator<'a> {
    /// Creates a public project generator for validated input.
    pub const fn new(config: &'a ValidatedConfig) -> Self {
        Self { config }
    }

    /// Generates the public project without filesystem access.
    pub fn generate_in_memory(&self) -> Result<BTreeMap<PathBuf, Vec<u8>>, CodegenError> {
        MEMORY_OUTPUT.with(|output| *output.borrow_mut() = Some(BTreeMap::new()));
        let result = self.generate(Path::new(""));
        let files = MEMORY_OUTPUT.with(|output| output.borrow_mut().take().unwrap_or_default());
        result.map(|_| files)
    }

    /// Generates the public project at `output_dir`.
    pub fn generate(&self, output_dir: &Path) -> Result<GeneratedProject, CodegenError> {
        crate::brookesia::BrookesiaGenerator::new(self.config).generate(output_dir)
    }
}

/// Which motor-control backend a validated configuration requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorBackend {
    Pwm,
    Dshot,
    Bdshot,
}

/// Mutually exclusive motor-control backend feature flags.
pub const MUTUALLY_EXCLUSIVE_FLAG_GROUPS: &[&[&str]] = &[&["USE_DSHOT", "USE_BDSHOT"]];

/// A set of incompatible active feature flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantFlagConflict {
    pub group: &'static [&'static str],
    pub present: Vec<&'static str>,
}

/// Returns the requested motor-control backend with deterministic precedence.
pub fn motor_control_backend(active_flags: &[String]) -> MotorBackend {
    if active_flags.iter().any(|flag| flag == "USE_BDSHOT") {
        MotorBackend::Bdshot
    } else if active_flags.iter().any(|flag| flag == "USE_DSHOT") {
        MotorBackend::Dshot
    } else {
        MotorBackend::Pwm
    }
}

/// Reports every mutually exclusive feature group with two or more active flags.
pub fn detect_variant_flag_conflicts(active_flags: &[String]) -> Vec<VariantFlagConflict> {
    MUTUALLY_EXCLUSIVE_FLAG_GROUPS
        .iter()
        .filter_map(|group| {
            let present = group
                .iter()
                .copied()
                .filter(|flag| active_flags.iter().any(|active| active == flag))
                .collect::<Vec<_>>();
            (present.len() >= 2).then_some(VariantFlagConflict { group, present })
        })
        .collect()
}
