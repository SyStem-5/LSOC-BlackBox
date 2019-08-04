use std::fs::File;
use std::io::prelude::Write;
use std::process::Command;
use std::thread;
use std::time;

use crate::mqtt::AsyncClient;

use crate::nodes::announce_blackbox_online;
use crate::settings::SettingsMosquitto;
use crate::INTERFACE_MQTT_USERNAME;

pub static REGISTERED_TOPIC: &str = "registered";
pub static UNREGISTERED_TOPIC: &str = "unregistered";
pub static WEBINTERFACE_TOPIC: &str = INTERFACE_MQTT_USERNAME;
pub static NEUTRONCOMMUNICATOR_TOPIC: &str = "neutron_communicators";

pub static TOPICS: &[&str] = &["registered/#", "unregistered/#", "external_interface/#", "neutron_communicators/#"];
pub static QOS: &[i32] = &[1, 1, 1, 1];

/**
 * Generates mosquitto(mqtt) configuration file.
 * Saved in <mqtt_conf_file_location>.
 */
pub fn generate_mosquitto_conf(settings: &SettingsMosquitto, restart_mqtt_docker_container: bool) {
    info!("Generating mosquitto broker configuration file...");

    let config_no_ssl = format!("listener 8883\n
persistence false\n
log_type all\n
log_dest syslog\n
allow_anonymous false\n
cafile /mosquitto/config/ca.crt\n
certfile /mosquitto/config/server.crt\n
keyfile /mosquitto/config/server.key\n
require_certificate false\n
auth_plugin /mosquitto/config/auth-plug.so\n
auth_opt_backends postgres\n
auth_opt_host database_postgres\n
auth_opt_port {}\n
auth_opt_dbname {}\n
auth_opt_user {}\n
auth_opt_pass {}\n
auth_opt_userquery SELECT password FROM mqtt_users WHERE username = $1 limit 1\n
auth_opt_superquery SELECT COALESCE(COUNT(*),0) FROM mqtt_users WHERE username = $1 AND superuser = 1\n
auth_opt_aclquery SELECT topic FROM mqtt_acl WHERE (username = $1) AND (rw >= $2)",
settings.db_port, settings.db_name, settings.db_username, settings.db_password);

    let mut file = File::create(settings.mosquitto_conf_save_location.to_string()).unwrap();
    file.write_all(&format!("{}", config_no_ssl).as_bytes())
        .unwrap();

    if restart_mqtt_docker_container {
        restart_mqtt_container();
    }

    info!(
        "Generated MQTT config. Location: {}",
        settings.mosquitto_conf_save_location
    );
}

/**
 * Restarts mosquitto broker docker container with "sudo docker restart <container_id>".
 */
fn restart_mqtt_container() {
    // First get container id so we can restart
    let container_id = Command::new("sudo")
        .arg("docker")
        .arg("ps")
        .arg("-a")
        .arg("-q")
        .arg("--filter")
        .arg("ancestor=mosquitto")
        .output()
        .expect("Failed to get mosquitto docker container id.");

    let out = Command::new("sudo")
        .arg("docker")
        .arg("restart")
        .arg(
            String::from_utf8_lossy(&container_id.stdout)
                .to_string()
                .replace("\n", ""),
        )
        .output()
        .expect("Failed to restart mosquitto docker container.");

    debug!("Restarting mqtt container.");

    if !String::from_utf8_lossy(&out.stderr).to_owned().is_empty() {
        error!(
            "Could not restart MQTT broker docker container. {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/**
 * OnConnectionSuccess mqtt callback.
 */
pub fn on_mqtt_connect_success(cli: &AsyncClient, _msgid: u16) {
    info!("Connection succeeded.");

    info!("Subscribing to: {}", TOPICS[0]);
    info!("Subscribing to: {}", TOPICS[2]);
    info!("Subscribing to: {}", TOPICS[3]);

    announce_blackbox_online(cli);

    cli.subscribe(TOPICS[0], QOS[0]);
    cli.subscribe(TOPICS[2], QOS[2]);
    cli.subscribe(TOPICS[3], QOS[3]);
}

/**
 * OnConnectionFail mqtt callback.
 */
pub fn on_mqtt_connect_failure(cli: &AsyncClient, _msgid: u16, rc: i32) {
    debug!("Connection attempt failed with error code {}.", rc);

    thread::sleep(time::Duration::from_millis(2500));
    cli.reconnect_with_callbacks(on_mqtt_connect_success, on_mqtt_connect_failure);
}

/**
 * OnConnectionLost mqtt callback.
 */
pub fn on_mqtt_connection_lost(cli: &AsyncClient) {
    error!("Connection lost. Reconnecting...");

    thread::sleep(time::Duration::from_millis(2500));
    cli.reconnect_with_callbacks(on_mqtt_connect_success, on_mqtt_connect_failure);
}
