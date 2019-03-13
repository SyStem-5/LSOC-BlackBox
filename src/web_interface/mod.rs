pub mod structures;

use crate::mqtt::{AsyncClient, Message};

use crate::mqtt_broker_manager::WEBINTERFACE_TOPIC;

fn new_command(command: structures::CommandType, data: &str) -> structures::Command {
    structures::Command {
        command,
        data: data.to_string(),
    }
}

/**
 * Sends data to WebInterface about an unregistered node from unregistered announces when discovery is enabled.
 */
pub fn wi_add_to_unregistered_list(cli: &AsyncClient, data: &str) {
    let msg = Message::new(
        WEBINTERFACE_TOPIC,
        serde_json::to_string(&new_command(
            structures::CommandType::AddToUnregisteredList,
            data,
        ))
        .unwrap(),
        1,
    );
    cli.publish(msg);
}

/**
 * Sends a registered node-element list to WebInterface topic.
 */
pub fn node_element_response(cli: &AsyncClient, data: Vec<structures::NodeFiltered>) {
    match serde_json::to_string(&data) {
        Ok(data_json) => {
            let msg = Message::new(
                WEBINTERFACE_TOPIC,
                serde_json::to_string(&new_command(
                    structures::CommandType::NodeElementList,
                    &data_json,
                ))
                .unwrap(),
                1,
            );

            cli.publish(msg);
        }
        Err(e) => error!(
            "Could not convert node_element data from database to json. {}",
            e
        ),
    }
}

/**
 * Send a 'announce' command to notify WebInterface that BlackBox is online if ```status``` parameter is true.
 * If its false then we just return a Message mqtt struct with the command payload, used for setting the MQTT last will for BlackBox.
 */
pub fn wi_announce_blackbox(cli: &AsyncClient, status: bool) -> Message {
    if status {
        let msg = Message::new(
            WEBINTERFACE_TOPIC,
            serde_json::to_string(&new_command(structures::CommandType::AnnounceOnline, ""))
                .unwrap(),
            2,
        );
        cli.publish(msg);
    } else {
        return Message::new(
            WEBINTERFACE_TOPIC,
            serde_json::to_string(&new_command(structures::CommandType::AnnounceOffline, ""))
                .unwrap(),
            2,
        );
    }

    return Message::new("", "", 1);
}

/**
 * Sent to the WebInterface when a module goes online or offline
 */
pub fn wi_noify_node_status(cli: &AsyncClient, node_identifier: &str, status: bool) {
    if status {
        let msg = Message::new(
            WEBINTERFACE_TOPIC,
            serde_json::to_string(&new_command(
                structures::CommandType::NodeOnline,
                node_identifier,
            ))
            .unwrap(),
            1,
        );
        cli.publish(msg);
    } else {
        let msg = Message::new(
            WEBINTERFACE_TOPIC,
            serde_json::to_string(&new_command(
                structures::CommandType::NodeOffline,
                node_identifier,
            ))
            .unwrap(),
            1,
        );
        cli.publish(msg);
    }
}
