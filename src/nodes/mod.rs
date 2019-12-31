use std::io::{Error, ErrorKind};

use paho_mqtt::AsyncClient;

use serde_json::from_str;

use r2d2::Pool;
use r2d2_postgres::{PostgresConnectionManager, postgres::NoTls};

use crate::credentials::{
    generate_mqtt_hash,
    generate_mqtt_password,
    generate_username
};
use crate::db_manager::{
    add_elements_to_element_table,
    add_node_to_node_table,
    remove_elements_from_elements_table,
    remove_from_unregistered_table,
    remove_node_from_node_table,
};
use crate::db_manager::{
    add_node_to_mqtt_acl, add_node_to_mqtt_users, remove_from_mqtt_users, remove_node_from_mqtt_acl,
};

mod commands;
pub use commands::{
    unregistered_notify,
    announce_blackbox_online,
    restart_node,
    announce_discovery,
    element_set
};

mod structs;
pub use structs::{Node, Element};

// THESE ARE TEMPORARY -- REMOVE WHEN FIXING THE MESS THAT IS THE MAIN FUNCTION
pub use structs::{Command, CommandType, ElementSummaryListItem};

// Used when generating a username for nodes
const REGISTERED_NODE_PREFIX: &str = "reg";


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
    db_conn_pool: Pool<PostgresConnectionManager<NoTls>>,
) -> Result<(), Error> {
    // Used to parse the data from WebInterface about a new node for registration
    #[derive(Debug, Serialize, Deserialize)]
    struct NewNodeJSON {
        pub identifier: String,
        pub name: String,
        pub elements: Vec<ElementJSON>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct ElementJSON {
        pub address: String,
        pub name: String,
        pub element_type: structs::ElementType,
        pub zone: String,
    }

    let node_json_object: NewNodeJSON;
    match from_str(node_data) {
        Ok(json) => node_json_object = json,
        Err(e) => {
            let msg = format!("Could not serialize unregistered node data. {}", e);

            return Err(Error::new(ErrorKind::InvalidData, msg));
        }
    }

    // Used for saving an Element list so we can add each to db
    let mut element_list: Vec<structs::Element> = Vec::new();

    // Generating username(identifier), MQTT password and hash
    let node_mqtt_password = generate_mqtt_password();
    let mqtt_password_hash = generate_mqtt_hash(&node_mqtt_password);

    let generated_identifier = [REGISTERED_NODE_PREFIX, &generate_username()].concat();

    if commands::send_credentials(mqtt_cli, &node_json_object.identifier, &generated_identifier, &node_mqtt_password).is_err() {
        return Err(Error::new(ErrorKind::Other, "Could not send credentials to the newly generated node."));
    }

    // Iterate over elements and populate the element_list
    for elem in node_json_object.elements {
        element_list.push(structs::Element {
            node_identifier: generated_identifier.to_owned(),
            address: elem.address,
            name: elem.name,
            element_type: elem.element_type,
            zone: elem.zone,
            data: default_element_data(elem.element_type),
        });
    }

    add_elements_to_element_table(db_conn_pool.clone(), element_list);

    // Create a node object
    let new_node = structs::Node::new(
        &generated_identifier,
        &node_json_object.name,
    );

    add_node_to_node_table(db_conn_pool.clone(), new_node);

    // Add node credentials to mqtt users list and to access control list
    add_node_to_mqtt_users(
        db_conn_pool.clone(),
        &generated_identifier,
        &mqtt_password_hash,
    );
    add_node_to_mqtt_acl(db_conn_pool.clone(), false, &generated_identifier);

    info!(
        "Successfully registered a new node. ID: {}",
        &generated_identifier
    );

    remove_from_unregistered_table(&node_json_object.identifier, db_conn_pool);

    Ok(())
}

/**
 * Removes the node information from the database - preventing it from accessing/using it.
 */
pub fn unregister_node(
    node_identifier: &str,
    db_conn_pool: Pool<PostgresConnectionManager<NoTls>>,
) -> bool {

    if !remove_from_mqtt_users(db_conn_pool.clone(), &node_identifier) {
        error!("Could not remove node from mqtt users table.");
        return false
    }

    if !remove_node_from_mqtt_acl(db_conn_pool.clone(), &node_identifier) {
        error!("Could not remove node from mqtt acl table.");
        return false
    }

    if !remove_elements_from_elements_table(db_conn_pool.clone(), &node_identifier) {
        error!("Could not remove element from the elements table.");
        return false
    }

    if !remove_node_from_node_table(db_conn_pool, &node_identifier) {
        error!("Could not remove node from node table.");
        return false
    }

    true
}

/**
 * Returns the default data string for database entry.
 * This is useful becase the ExternalInterface can have some data to base the UI from rather than none.
 */
fn default_element_data(element_type: structs::ElementType) -> String {
    let d = match element_type {
        structs::ElementType::BasicSwitch => "false",
        structs::ElementType::DHT11 => "{\"temp\": \"0\", \"hum\": \"0\"}",
        structs::ElementType::Thermostat => "0"
    };

    String::from(d)
}
