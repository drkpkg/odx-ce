use configparser::ini::Ini;
use crate::utils::find_project_root;

#[allow(dead_code)]
pub struct Config {
    pub db_host: String,
    pub db_port: String,
    pub db_user: String,
    pub db_password: String,
    pub db_name: Option<String>,
}

impl Config {
    #[allow(dead_code)]
    pub fn load() -> Result<Self, String> {
        let project_root = find_project_root()?;

        let config_file = project_root.join("odoo.conf.local");
        let config_file = if config_file.exists() {
            config_file
        } else {
            project_root.join("odoo.conf")
        };

        if !config_file.exists() {
            return Err(format!("Config file not found: {}", config_file.display()));
        }

        let mut config = Ini::new();
        config.load(&config_file).map_err(|e| format!("Failed to load config: {}", e))?;

        let db_host = config.get("options", "db_host")
            .unwrap_or_else(|| "localhost".to_string());
        let db_port = config.get("options", "db_port")
            .unwrap_or_else(|| "5432".to_string());
        let db_user = config.get("options", "db_user")
            .unwrap_or_else(|| "odoo".to_string());
        let db_password = config.get("options", "db_password")
            .unwrap_or_else(|| "odoo".to_string());
        let db_name = config.get("options", "db_name");

        Ok(Config {
            db_host,
            db_port,
            db_user,
            db_password,
            db_name,
        })
    }
}
