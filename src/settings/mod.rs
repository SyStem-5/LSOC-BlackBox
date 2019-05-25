use std::{
    fs::{create_dir_all, File},
    io::prelude::Read,
    io::Error,
    io::Write,
    path::Path,
};

use serde_json::{from_str, to_string};

use crate::credentials::{generate_mqtt_password, get_db_password_from_file};

use crate::APP_VERSION;

const BASE_DIRECTORY: &str = "/etc/BlackBox/";
const SETTINGS_FILE_LOCATION: &str = "/etc/BlackBox/settings.json";
const VERSION_FILE_LOCATION: &str = "/etc/BlackBox/blackbox.version";

const SETTINGS_DEFAULT: &str = r#"{
    "mosquitto_broker_config": {
        "mosquitto_conf_save_location": "/etc/mosquitto/mosquitto.conf",
        "db_ip": "127.0.0.1",
        "db_port": "5432",
        "db_username": "postgres",
        "db_password": "",
        "db_name": "postgres"
    },
    "database_settings": {
        "db_ip": "127.0.0.1",
        "db_port": "5432",
        "db_username": "postgres",
        "db_password": "",
        "db_name": "postgres"
    },
    "blackbox_mqtt_client": {
        "mqtt_ip": "127.0.0.1",
        "mqtt_port": "8883",
        "mqtt_password": ""
    },
    "nodes": {
        "mqtt_unregistered_node_password": "unregistered"
    },
    "external_interface_settings": {
        "mqtt_password": ""
    }
}"#;

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub mosquitto_broker_config: SettingsMosquitto,

    pub database_settings: SettingsDatabase,

    pub blackbox_mqtt_client: SettingsMqttClient,

    pub nodes: SettingsNodes,

    pub external_interface_settings: SettingsWebInterface,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsMosquitto {
    pub mosquitto_conf_save_location: String,
    pub db_ip: String,
    pub db_port: String,
    pub db_username: String,
    pub db_password: String,
    pub db_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsMqttClient {
    pub mqtt_ip: String,
    pub mqtt_port: String,
    pub mqtt_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsNodes {
    pub mqtt_unregistered_node_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsDatabase {
    pub db_ip: String,
    pub db_port: String,
    pub db_username: String,
    pub db_password: String,
    pub db_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsWebInterface {
    pub mqtt_password: String,
}

/**
 * Checks if the settings file exists.
 * If it exists, try to load and return it.
 * If it exists, but fails to load, log error message and exit.
 * If it doesn't exist return Err to main.
 */
pub fn init() -> Result<Settings, ()> {
    refresh_version_file();

    if !Path::new(SETTINGS_FILE_LOCATION).exists() {
        error!("Settings file not found.");
        info!("Run 'sudo black_box commands' to get the command for generating a settings file.");
        return Err(());
    } else {
        match load_settings() {
            Ok(settings) => {
                info!("Settings loaded successfully.");
                return Ok(settings);
            }
            Err(e) => {
                error!("Failed to load settings file. {}", e);
                return Err(());
            }
        }
    }
}

/**
 * Generates/rewrites a JSON file, containing the current version number, in the settings base directory.
 * This file is primarily used by Neutron Update Client.
 */
fn refresh_version_file() {
    debug!("Generating version file...");

    //let v_file = format!("{{\"version\": \"{}\"}}", APP_VERSION);

    match create_dir_all(BASE_DIRECTORY) {
        Ok(_) => match File::create(VERSION_FILE_LOCATION) {
            Ok(mut file) => match file.write_all(APP_VERSION.as_bytes()) {
                Ok(_) => {
                    debug!("Version file generated.");
                }
                Err(e) => error!(
                    "Could not generate version file. Failed to write to file. {}",
                    e
                ),
            },
            Err(e) => error!(
                "Could not generate version file. Failed to create file. {}",
                e
            ),
        },
        Err(e) => error!(
            "Could not generate version file. Failed to create folder. {}",
            e
        ),
    }
}

/**
 * Creates a settings file and saves the default settings in it.
 * Returns Settings object if successfull.
 */
pub fn write_default_settings() -> Result<(), Error> {
    info!("Generating default settings file...");

    let mut file = File::create(SETTINGS_FILE_LOCATION)?;
    file.write_all(SETTINGS_DEFAULT.as_bytes())?;

    info!(
        "Default settings file generated. Only root can modify the file. Location: {}",
        SETTINGS_FILE_LOCATION
    );

    Ok(())
}

/**
 * Tries to load the settings file.
 * Returns Settings object if successfull.
 */
fn load_settings() -> Result<Settings, Error> {
    info!("Loading settings file: '{}'", SETTINGS_FILE_LOCATION);

    let mut contents = String::new();

    let mut file = File::open(SETTINGS_FILE_LOCATION)?;

    file.read_to_string(&mut contents)?;

    let mut settings: Settings = from_str(&contents)?;

    // If the database password field is empty, try to load from file by calling <credentials>
    if settings.database_settings.db_password.is_empty() {
        match get_db_password_from_file() {
            Ok(pass) => {
                settings.database_settings.db_password = pass.to_owned();

                if settings.mosquitto_broker_config.db_password.is_empty() {
                    info!("Mosquitto broker database password found empty, setting to BlackBox database password.");
                    settings.mosquitto_broker_config.db_password = pass;
                }
            }
            Err(_) => {
                error!("Database password not found, please contact the system owner.");
                return Err(Error::new(std::io::ErrorKind::InvalidData, ""));
            }
        }
    }

    if settings.blackbox_mqtt_client.mqtt_password.is_empty() {
        info!("BlackBox MQTT password not found. Generating and saving new password.");
        let password = generate_mqtt_password();

        // Just in case we are going to startup, set the password in the struct
        settings.blackbox_mqtt_client.mqtt_password = password.to_string();

        // Save to settings file
        match save_mqtt_password(&password) {
            Ok(_) => {}
            Err(e) => {
                error!("Could not save BlackBox mqtt password to file. {}", e);
                return Err(Error::new(std::io::ErrorKind::InvalidData, ""));
            }
        }
    }

    if settings.nodes.mqtt_unregistered_node_password.is_empty() {
        warn!("Unregistered node password field is empty. Using default password: 'unregistered'.");
        settings.nodes.mqtt_unregistered_node_password = "unregistered".to_string();
    }

    if settings
        .external_interface_settings
        .mqtt_password
        .is_empty()
    {
        info!("Web Interface is enabled but its password is not set. Generating and saving new password..");
        let password = generate_mqtt_password();

        settings.external_interface_settings.mqtt_password = password.to_string();

        // Save to settings file
        match save_web_interface_password(&password) {
            Ok(_) => {}
            Err(e) => {
                error!("Could not save WebInterface mqtt password to file. {}", e);
                return Err(Error::new(std::io::ErrorKind::InvalidData, ""));
            }
        }
    }

    Ok(settings)
}

/**
 * Saves mqtt password in the settings file for the BlackBox client.
 */
fn save_mqtt_password(password: &str) -> Result<(), Error> {
    let mut contents = String::new();

    let mut file = File::open(SETTINGS_FILE_LOCATION)?;

    file.read_to_string(&mut contents)?;

    let mut settings: Settings = from_str(&contents)?;

    settings.blackbox_mqtt_client.mqtt_password = String::from(password);

    let mut file = File::create(SETTINGS_FILE_LOCATION)?;
    file.write_all(&format!("{}", to_string(&settings)?).as_bytes())?;

    Ok(())
}

/**
 * Saves Web Interface mqtt password in the settings file for setting up mqtt access.
 */
fn save_web_interface_password(password: &str) -> Result<(), Error> {
    let mut contents = String::new();

    let mut file = File::open(SETTINGS_FILE_LOCATION)?;

    file.read_to_string(&mut contents)?;

    let mut settings: Settings = from_str(&contents)?;

    settings.external_interface_settings.mqtt_password = String::from(password);

    let mut file = File::create(SETTINGS_FILE_LOCATION)?;
    file.write_all(&format!("{}", to_string(&settings)?).as_bytes())?;

    Ok(())
}
