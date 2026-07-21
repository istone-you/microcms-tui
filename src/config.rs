use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileConfig {
    pub service_id: Option<String>,
    pub api_key: Option<String>,
    pub default_endpoint: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub service_id: Option<String>,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigOverrides {
    pub service_id: Option<String>,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("", "", "microcms-tui")
        .context("could not determine the platform config directory")?;
    Ok(project_dirs.config_dir().join("config.toml"))
}

pub fn load_file_config() -> Result<FileConfig> {
    let path = config_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileConfig::default())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };

    toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_file_config(config: &FileConfig) -> Result<()> {
    let path = config_path()?;
    let parent = path
        .parent()
        .context("config path does not have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let contents = toml::to_string_pretty(config).context("failed to serialize config")?;
    fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn effective_config(file: FileConfig, env: ConfigOverrides, cli: ConfigOverrides) -> Config {
    Config {
        service_id: cli.service_id.or(env.service_id).or(file.service_id),
        api_key: cli.api_key.or(env.api_key).or(file.api_key),
        endpoint: cli.endpoint.or(env.endpoint).or(file.default_endpoint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_config_uses_cli_then_env_then_file_for_all_fields() {
        let file = FileConfig {
            service_id: Some("file-service".into()),
            api_key: Some("file-key".into()),
            default_endpoint: Some("file-endpoint".into()),
        };
        let env = ConfigOverrides {
            service_id: Some("env-service".into()),
            api_key: Some("env-key".into()),
            endpoint: Some("env-endpoint".into()),
        };
        let cli = ConfigOverrides {
            service_id: Some("cli-service".into()),
            api_key: Some("cli-key".into()),
            endpoint: Some("cli-endpoint".into()),
        };

        assert_eq!(
            effective_config(file, env, cli),
            Config {
                service_id: Some("cli-service".into()),
                api_key: Some("cli-key".into()),
                endpoint: Some("cli-endpoint".into()),
            }
        );
    }

    #[test]
    fn effective_config_falls_back_field_by_field() {
        let file = FileConfig {
            service_id: Some("file-service".into()),
            api_key: Some("file-key".into()),
            default_endpoint: Some("file-endpoint".into()),
        };
        let env = ConfigOverrides {
            service_id: None,
            api_key: Some("env-key".into()),
            endpoint: None,
        };
        let cli = ConfigOverrides {
            service_id: Some("cli-service".into()),
            api_key: None,
            endpoint: None,
        };

        assert_eq!(
            effective_config(file, env, cli),
            Config {
                service_id: Some("cli-service".into()),
                api_key: Some("env-key".into()),
                endpoint: Some("file-endpoint".into()),
            }
        );
    }
}
