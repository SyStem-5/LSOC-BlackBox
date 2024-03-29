pub mod structs;

use crate::mqtt::{AsyncClient, Message};

use crate::mqtt_broker_manager::WEBINTERFACE_TOPIC;

fn new_command(command: structs::CommandType, data: &str) -> structs::Command {
    structs::Command {
        command,
        data: data.to_string(),
    }
}

/**
 * Sends data to ExtInterface about an unregistered node from unregistered announces when discovery is enabled.
 */
pub fn add_to_unregistered_list(cli: &AsyncClient, data: &str) {
    let msg = Message::new(
        WEBINTERFACE_TOPIC,
        serde_json::to_string(&new_command(
            structs::CommandType::AddToUnregisteredList,
            data,
        ))
        .unwrap(),
        1,
    );
    cli.publish(msg);
}

/**
 * Sends data to ExtInterface about an unregistered node that went offline.
 */
pub fn remove_from_unregistered_list(cli: &AsyncClient, data: &str) {
    let msg = Message::new(
        WEBINTERFACE_TOPIC,
        serde_json::to_string(&new_command(
            structs::CommandType::RemoveFromUnregisteredList,
            data,
        ))
        .unwrap(),
        1,
    );
    cli.publish(msg);
}

/**
 * Sends a registered node-element list to ExtInterface topic.
 */
pub fn node_element_response(cli: &AsyncClient, data: Vec<structs::NodeFiltered>) {
    match serde_json::to_string(&data) {
        Ok(data_json) => {
            let msg = Message::new(
                WEBINTERFACE_TOPIC,
                serde_json::to_string(&new_command(
                    structs::CommandType::NodeElementList,
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
 * Send a 'announce' command to notify ExtInterface that BlackBox is online if `status` parameter is true.
 * If its false then we just return a Message mqtt struct with the command payload, used for setting the MQTT last will for BlackBox.
 */
pub fn announce_blackbox(cli: &AsyncClient, status: bool) -> Message {
    let cmd = if status {
        structs::CommandType::AnnounceOnline
    } else {
        structs::CommandType::AnnounceOffline
    };

    let msg = Message::new(
        WEBINTERFACE_TOPIC,
        serde_json::to_string(&new_command(cmd, ""))
            .unwrap(),
        2,
    );

    if status {
        cli.publish(msg.clone());
    }

    msg
}

/**
 * Sent to the ExtInterface when a module changes states.
 */
pub fn node_status(cli: &AsyncClient, node_identifier: &str, status: &str) {
    match serde_json::to_string(&new_command(structs::CommandType::NodeStatus, &[node_identifier, "::", status].concat())) {
        Ok(json) => {
            let msg = Message::new(WEBINTERFACE_TOPIC, json, 1);
            cli.publish(msg);
        }
        Err(e) => error!("Could not parse the Command struct to string. {}", e)
    }
}

pub fn element_state(cli: &AsyncClient, node_identifier: &str, data: &str) {
    match serde_json::to_string(&new_command(structs::CommandType::UpdateElementState, &[node_identifier, "::", data].concat())) {
        Ok(json) => {
            let msg = Message::new(WEBINTERFACE_TOPIC, json, 1);
            cli.publish(msg);
        }
        Err(e) => error!("Could not parse the Command struct to string. {}", e)
    }
}

/**
 * Sends data to ExtInterface about the newly registered node.
 * This is just so that the WI knows that this node was successfully registered.
 */
pub fn node_registered(cli: &AsyncClient, data: &str) {
    let msg = Message::new(
        WEBINTERFACE_TOPIC,
        serde_json::to_string(&new_command(
            structs::CommandType::NodeRegistration,
            data,
        ))
        .unwrap(),
        1,
    );
    cli.publish(msg);
}
