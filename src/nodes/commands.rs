use std::io::{Error, ErrorKind};

use paho_mqtt::{AsyncClient, Message};

use serde_json::from_str;

use crate::external_interface::announce_blackbox;

use crate::mqtt_broker_manager::{REGISTERED_TOPIC, UNREGISTERED_TOPIC};

use super::structs::{Command, CommandType};

/**
 * Converts the `Command` struct to a string then publishes it to the nodes topic.
 * QoS valid values are: `0, 1, 2`
 * If there is a problem parsing the `Command` struct the function returns an error.
 */
pub fn send_node_command(
    mqtt_cli: &AsyncClient,
    cmd: CommandType,
    node_id: &str,
    data: Option<&str>,
    qos: i32,
) -> Result<(), Error> {
    if let Some(cmd) = Command::new(cmd, data.unwrap_or_default()).to_string() {
        let msg = Message::new([REGISTERED_TOPIC, "/", node_id].concat(), cmd, qos);
        mqtt_cli.publish(msg);

        return Ok(());
    }

    Err(Error::new(ErrorKind::Other, "Could not send MQTT message"))
}

/**
 * Publishes an 'Announce' message to unregistered topic.
 */
pub fn announce_discovery(mqtt_cli: &AsyncClient) {
    if let Some(cmd) = Command::new(CommandType::Announce, "").to_string() {
        let msg = Message::new(UNREGISTERED_TOPIC, cmd, 1);
        mqtt_cli.publish(msg);
    }
}

/**
 * Publishes an 'Announce' message to registered and WebInterface topics.
 */
pub fn announce_blackbox_online(mqtt_cli: &AsyncClient) {
    if let Some(cmd) = Command::new(CommandType::Announce, "").to_string() {
        let msg = Message::new(REGISTERED_TOPIC, cmd, 1);
        mqtt_cli.publish(msg);
    }

    // Tell ExternalInterface that BlackBox is online
    announce_blackbox(mqtt_cli, true);
}

/**
 *
 */
pub fn restart_node(mqtt_cli: &AsyncClient, node_id: &str) -> Result<(), Error> {
    send_node_command(mqtt_cli, CommandType::RestartDevice, node_id, None, 2)
}

/**
 *
 */
pub fn unregistered_notify(mqtt_cli: &AsyncClient, node_id: &str) -> Result<(), Error> {
    send_node_command(mqtt_cli, CommandType::UnregisterNotify, node_id, None, 2)
}

/**
 *
 */
pub fn element_set(mqtt_cli: &AsyncClient, data: &str) -> Result<(), Error> {
    #[derive(Serialize)]
    struct ElementData {
        id: String,
        data: String,
    }

    match from_str::<super::structs::ElementSet>(data) {
        Ok(res) => {
            let ser_data = ElementData {
                id: res.element_identifier,
                data: res.data,
            };

            send_node_command(
                mqtt_cli,
                CommandType::SetElementState,
                &res.node_identifier,
                Some(&serde_json::to_string(&ser_data).unwrap_or_default()),
                1,
            )
        }
        Err(e) => {
            Err(Error::new(
                ErrorKind::InvalidData,
                format!("Could not parse data. {}", e),
            ))
        }
    }
}

/**
 *
 *
 * Sent to unregistered nodes.
 */
pub fn send_credentials(
    mqtt_cli: &AsyncClient,
    unreged_id: &str,
    reged_id: &str,
    new_pass: &str,
) -> Result<(), ()> {
    let payload = [reged_id, ":", new_pass].concat();

    if let Some(cmd) = Command::new(CommandType::ImplementCreds, &payload).to_string() {
        let topic = [UNREGISTERED_TOPIC, "/", unreged_id].concat();

        let msg = Message::new(topic, cmd, 2);
        mqtt_cli.publish(msg);

        return Ok(());
    }

    Err(())
}
