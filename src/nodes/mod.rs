use crate::mqtt::{AsyncClient, Message};

use serde_json::{from_str, to_string, Error};

use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;

use crate::credentials::{generate_mqtt_hash, generate_mqtt_password, generate_username};
use crate::db_manager::{
    add_elements_to_element_table, add_node_to_node_table, create_node_object,
    remove_elements_from_elements_table, remove_from_unregistered_table,
    remove_node_from_node_table,
};
use crate::db_manager::{
    add_node_to_mqtt_acl, add_node_to_mqtt_users, remove_from_mqtt_users, remove_node_from_mqtt_acl,
};
use crate::db_manager::{Element, ElementSummaryListItem};
use crate::mqtt_broker_manager::{REGISTERED_TOPIC, UNREGISTERED_TOPIC};
use crate::web_interface::wi_announce_blackbox;

// Used when generating a username for nodes
const REGISTERED_NODE_PREFIX: &str = "reg";

//Supported element types
#[derive(Clone, ToString, Debug, Serialize, Deserialize)]
pub enum ElementType {
    BasicSwitch,
    DHT11,
}

// Used to parse the data from WebInterface about a new node for registration
#[derive(Debug, Serialize, Deserialize)]
pub struct NewNodeJSON {
    pub unreged_id: String,
    pub node_name: String,
    pub node_category: String,
    pub elements: Vec<Element>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    pub command: CommandType,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize, ToString, PartialEq)]
pub enum CommandType {
    Announce,           // Sent to all unregistered nodes to request element lists
    AnnounceState,      // Received when nodes state changes
    ImplementCreds,     // Sent to node when the creds are being sent
    UnregisterNotify,   // Sent to node when it gets unregistered
    ElementSummary,     // Received from node with the element summary list
    SetElementState,    // Sent to node to set the element state
    UpdateElementState, // Recieved from node
    RestartDevice,      // Sent to node
}

pub fn new_command(command: CommandType, data: &str) -> Command {
    Command {
        command,
        data: data.to_string(),
    }
}

/**
 * Registers node - adds needed info to db, sends the creds to the unregistered node and removes from unregistered node list.
 *
 * `node_data` - type NewNodeJson (in json format).
 *
 * Returns:
 * * `Touple.0` - node identifier
 * * `Touple.1` - node hashed password
 */
pub fn register_node(
    node_data: &str,
    mqtt_cli: &AsyncClient,
    db_conn_pool: Pool<PostgresConnectionManager>,
) -> Result<(), Error> {
    //
    let node_json_object: NewNodeJSON = from_str(node_data)?;

    // Used for saving an Element list so we can add each to db
    let mut element_list: Vec<Element> = Vec::new();

    // Used for saving a summary of elements available to save to node_table
    let mut element_summary_list: Vec<ElementSummaryListItem> = Vec::new();

    // Generating username(identifier), MQTT password and hash
    let generated_identifier;
    let node_mqtt_password = generate_mqtt_password();
    let mqtt_password_hash = generate_mqtt_hash(&node_mqtt_password);

    generated_identifier = format!("{}{}", REGISTERED_NODE_PREFIX, generate_username());

    // Iterate over elements and populate the element_list and element_summary_list
    for elem in node_json_object.elements {
        element_list.push(Element {
            node_id: generated_identifier.to_string(),
            address: elem.address.to_string(),
            name: elem.name.to_string(),
            element_type: elem.element_type.clone(),
            data: None,
        });
        element_summary_list.push(ElementSummaryListItem {
            address: elem.address.to_string(),
            element_type: elem.element_type.clone(),
        });
    }

    // Add node credentials to mqtt users list and to access control list
    add_node_to_mqtt_users(
        db_conn_pool.clone(),
        &generated_identifier,
        &mqtt_password_hash,
    );
    add_node_to_mqtt_acl(db_conn_pool.clone(), false, &generated_identifier);

    // Create a node object
    let new_node = create_node_object(
        &generated_identifier,
        &node_json_object.node_name,
        &node_json_object.node_category,
        &to_string(&element_summary_list).unwrap(),
    );

    add_elements_to_element_table(db_conn_pool.clone(), element_list);

    add_node_to_node_table(db_conn_pool.clone(), new_node);

    info!(
        "Successfully registered a new node. ID: {}",
        &generated_identifier
    );

    // Send mqtt credentials to the new node
    let payload = format!("{},{}", generated_identifier, node_mqtt_password);
    let msg = Message::new(
        UNREGISTERED_TOPIC.to_string() + &"/" + &node_json_object.unreged_id,
        to_string(&new_command(CommandType::ImplementCreds, &payload)).unwrap(),
        1,
    );
    mqtt_cli.publish(msg);

    remove_from_unregistered_table(&node_json_object.unreged_id, db_conn_pool);

    Ok(())
}

/**
 * Unregisteres node with node_identifier and sends a command to the node, notifying it.
 */
pub fn unregister_node(
    node_identifier: &str,
    mqtt_cli: &AsyncClient,
    db_conn_pool: Pool<PostgresConnectionManager>,
) -> bool {
    // Send mqtt message to node
    let msg = Message::new(
        REGISTERED_TOPIC.to_string() + &"/" + &node_identifier,
        to_string(&new_command(CommandType::UnregisterNotify, "")).unwrap(),
        2,
    );
    mqtt_cli.publish(msg);

    let mqtt_users = remove_from_mqtt_users(db_conn_pool.clone(), &node_identifier);

    let mqtt_acl = remove_node_from_mqtt_acl(db_conn_pool.clone(), &node_identifier);

    let blackbox_elements =
        remove_elements_from_elements_table(db_conn_pool.clone(), &node_identifier);

    let blackbox_nodes = remove_node_from_node_table(db_conn_pool.clone(), &node_identifier);

    if mqtt_users && mqtt_acl && blackbox_elements && blackbox_nodes {
        info!("Node unregistered successfully. ID: {}", &node_identifier);
        return true;
    } else {
        error!("Failed to unregister node. ID: {}", &node_identifier);
        return false;
    }
}

/**
 * Publishes an 'Announce' message to unregistered topic.
 */
pub fn announce_discovery(mqtt_cli: &AsyncClient) {
    let msg = Message::new(
        UNREGISTERED_TOPIC,
        to_string(&new_command(CommandType::Announce, "")).unwrap(),
        1,
    );
    mqtt_cli.publish(msg);

    // if let Err(e) = tok.wait() {
    //     error!("Error sending announce message: {:?}", e);
    // }
}

/**
 * Publishes an 'Announce' message to registered and WebInterface topics.
 */
pub fn announce_blackbox_online(mqtt_cli: &AsyncClient) {
    let msg = Message::new(
        REGISTERED_TOPIC,
        to_string(&new_command(CommandType::Announce, "")).unwrap(),
        1,
    );
    mqtt_cli.publish(msg);

    wi_announce_blackbox(mqtt_cli, true);
}

/**
 * Converts the `Command` struct to a string then publishes it to the nodes topic.
 * QoS valid values are: 0, 1, 2
 * If there is a problem parsing the `Command` struct the function returns an error.
 */
pub fn send_node_command(mqtt_cli: &AsyncClient, cmd: CommandType, node_id: &str, qos: i32) -> Result<(), Error> {
    match to_string(&new_command(cmd, "")) {
        Ok(json) => {
            let msg = Message::new([REGISTERED_TOPIC, "/", node_id].concat(), json, qos);
            mqtt_cli.publish(msg);
        }
        Err(e) => return Err(e)
    }

    Ok(())
}
