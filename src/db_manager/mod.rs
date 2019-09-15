// Temporary
//#![allow(dead_code)]

use crate::credentials::generate_mqtt_hash;
use crate::mqtt_broker_manager::{QOS, TOPICS, NEUTRONCOMMUNICATOR_TOPIC, REGISTERED_TOPIC};
use crate::nodes::ElementType;
use crate::settings::{NeutronCommunicator, SettingsDatabase, SettingsWebInterface};
use crate::external_interface::structs::{ElementsFiltered, NodeFiltered, NodeInfoEdit};
use crate::{BLACKBOX_MQTT_USERNAME, INTERFACE_MQTT_USERNAME};

use postgres::params::{ConnectParams, Host};
use r2d2::Pool;
use r2d2_postgres::{PostgresConnectionManager, TlsMode};

const MQTT_USERNAME_UNREGISTERED: &str = "unregistered_node";

const TABLE_BLACKBOX_UNREGISTERED: &str = "blackbox_unregistered";
const TABLE_BLACKBOX_NODES: &str = "blackbox_nodes";
const TABLE_BLACKBOX_ELEMENTS: &str = "blackbox_elements";

const TABLE_MQTT_USERS: &str = "mqtt_users";
const TABLE_MQTT_ACL: &str = "mqtt_acl";

const MQTT_READ_WRITE: i32 = 3;
const MQTT_WRITE_ONLY: i32 = 2;
const MQTT_SUBSCRIBE_ONLY: i32 = 4;
const MQTT_READ_ONLY: i32 = 1;

// Used for unregistered node object in <TABLE_BLACKBOX_UNREGISTERED>
#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisteredNodeItem {
    pub client_id: String,
    pub elements_summary: String,
}

// Used for objects in <TABLE_BLACKBOX_NODES>
#[derive(Debug, Clone)]
pub struct Node {
    pub id: i64,
    pub identifier: String,
    pub name: String,
    pub category: String,
    pub elements_summary: String,
    pub state: bool,
}

// Not sure if this is going to be useful
// #[derive(Debug)]
// pub struct NodeState {
//     pub identifier: String,
//     pub state: bool,
// }

// Used in <TABLE_BLACKBOX_ELEMENTS>
#[derive(Debug, Serialize, Deserialize)]
pub struct Element {
    pub node_id: String,
    pub address: String,
    pub name: String,
    pub element_type: ElementType,
    pub data: Option<String>,
}

// Used for element_summary in <TABLE_BLACKBOX_NODES> field elements_summary
#[derive(Debug, Serialize, Deserialize)]
pub struct ElementSummaryListItem {
    pub address: String,
    pub element_type: ElementType,
}

/**
 * Creates a new Node object and returns it.
 */
pub fn create_node_object(
    identifier: &str,
    name: &str,
    category: &str,
    elements_summary: &str,
) -> Node {
    Node {
        id: 0,
        identifier: identifier.to_string(),
        name: name.to_string(),
        elements_summary: elements_summary.to_string(),
        category: category.to_string(),
        state: false,
    }
}

/**
 * Checks if the db contains necessary tables for operation.
 * If it doesn't, it creates them.
 *
 * This function creates a db connection and returns it.
 *
 * If the mosquitto broker and BlackBox are using the same db server,
 * and BlackBox creates tables for authentication,
 * the new password for BlackBox is returned.
 * ```
 * Touple.0 - Database connection pool
 * Touple.1 - New mqtt password(if empty, then ignore)
 * ```
 */
pub fn init(
    bb_mqtt_same_db: bool,
    db_settings: &SettingsDatabase,
    mqtt_blackbox_pass: &str,
    web_interface: &SettingsWebInterface,
    neutron_communicators: &[NeutronCommunicator],
) -> Result<(Pool<PostgresConnectionManager>), i8> {
    let builder = ConnectParams::builder()
        .port(db_settings.db_port.parse::<u16>().unwrap())
        .database(&db_settings.db_name)
        .user(
            &db_settings.db_username,
            if !db_settings.db_password.is_empty() {
                Some(&db_settings.db_password)
            } else {
                None
            },
        )
        .build(Host::Tcp(db_settings.db_ip.to_string()));

    let manager = PostgresConnectionManager::new(builder.clone(), TlsMode::None).unwrap();
    let manager_return = PostgresConnectionManager::new(builder, TlsMode::None).unwrap();

    let _pool = Pool::new(manager);
    match _pool {
        Ok(db) => {
            info!("Database connection successful.");

            // Check which table exists
            if !table_exists(TABLE_BLACKBOX_NODES, db.clone()) {
                info!("Creating BlackBox Nodes table.");
                if !create_table_nodes(db.clone()) {
                    return Err(1);
                }
            }
            if !table_exists(TABLE_BLACKBOX_ELEMENTS, db.clone()) {
                info!("Creating BlackBox Elements table.");
                if !create_table_elements(db.clone()) {
                    return Err(1);
                }
            }
            // Unregistered table is not supposed to be here on startup so we remove it if it exists
            if table_exists(TABLE_BLACKBOX_UNREGISTERED, db.clone()) {
                set_discovery_mode(false, "", db.clone(), None);
            }
            // If we're on the same db then create the tables, generate the mosquitto config and credentials
            if bb_mqtt_same_db {
                if !table_exists(TABLE_MQTT_USERS, db.clone()) {
                    info!("Creating MQTT Users table.");
                    if !create_table_mqtt_users(db.clone()) {
                        return Err(1);
                    }
                }
                if !table_exists(TABLE_MQTT_ACL, db.clone()) {
                    info!("Creating MQTT ACL table.");
                    if !create_table_mqtt_acl(db.clone()) {
                        return Err(1);
                    }
                }

                if !check_mqtt_user_exists(db.clone(), BLACKBOX_MQTT_USERNAME) {
                    info!("BlackBox MQTT credentials not found. Generating...");

                    if !set_mqtt_bb_creds(db.clone(), bb_mqtt_same_db, mqtt_blackbox_pass) {
                        return Err(1);
                    }
                }

                if !check_mqtt_user_exists(db.clone(), &INTERFACE_MQTT_USERNAME) {
                    info!("Interface MQTT credentials not found. Generating...");

                    if !set_external_interface_creds(
                        db.clone(),
                        bb_mqtt_same_db,
                        &web_interface.mqtt_password,
                    ) {
                        return Err(1);
                    }
                }

                for neutron_communicator in neutron_communicators {
                    if !neutron_communicator.mqtt_password.is_empty()
                        && !neutron_communicator.mqtt_username.is_empty()
                        && !check_mqtt_user_exists(db.clone(), &neutron_communicator.mqtt_username)
                    {
                        if !set_neutron_communicator_creds(
                            db.clone(),
                            &neutron_communicator.mqtt_username,
                            &neutron_communicator.mqtt_password,
                        ) {
                            return Err(1);
                        }
                    }
                }
            } else {
                info!("Running BlackBox and MQTT broker on different databases.");
            }

            // Set all node states to 0
            edit_node_state_global(false, db.clone());
        }
        Err(e) => error!("Database connection failed. {}", e),
    }

    info!("Database initialization complete.");

    Ok(Pool::new(manager_return).unwrap())
}

/**
 * Queries the database to check if a table exists.
 * Returns a boolean true if found.
 */
fn table_exists(table_name: &str, db_conn: Pool<PostgresConnectionManager>) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!("SELECT 1 FROM {} LIMIT 1;", table_name);

    let query = _conn.execute(&comm, &[]);
    match query {
        Ok(_) => true,
        Err(_) => false,
    }
}

/**
 * Generates <TABLE_BLACKBOX_NODES>.
 */
fn create_table_nodes(db_conn: Pool<PostgresConnectionManager>) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!(
        r"CREATE TABLE {} (
                        id                   SERIAL PRIMARY KEY,
                        identifier           TEXT NOT NULL,
                        name                 TEXT NOT NULL,
                        category             TEXT NOT NULL,
                        elements_summary     TEXT NOT NULL,
                        state                BOOLEAN NOT NULL
                        );",
        TABLE_BLACKBOX_NODES
    );

    let query = _conn.execute(&comm, &[]);

    match query {
        Ok(_) => return true,
        Err(e) => {
            error!("Failed to create table {}. {}", TABLE_BLACKBOX_NODES, e);
            return false;
        }
    }
}

/**
 * Generates <TABLE_BLACKBOX_ELEMENTS>.
 */
fn create_table_elements(db_conn: Pool<PostgresConnectionManager>) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!(
        r"CREATE TABLE {} (
                    node_id         TEXT NOT NULL,
                    address         TEXT NOT NULL,
                    name            TEXT NOT NULL,
                    element_type    TEXT NOT NULL,
                    data            TEXT NOT NULL
                    );",
        TABLE_BLACKBOX_ELEMENTS
    );

    let query = _conn.execute(&comm, &[]);

    match query {
        Ok(_) => return true,
        Err(e) => {
            error!("Failed to create table {}. {}", TABLE_BLACKBOX_ELEMENTS, e);
            return false;
        }
    }
}

/**
 *  Generates <MYSQL_MQTT_USERS_TABLE>.
 */
fn create_table_mqtt_users(db_conn: Pool<PostgresConnectionManager>) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!(
        r"CREATE TABLE {} (
                    id           BIGSERIAL PRIMARY KEY,
                    username     TEXT NOT NULL,
                    password     TEXT NOT NULL,
                    superuser    BOOLEAN NOT NULL DEFAULT FALSE
                    );",
        TABLE_MQTT_USERS
    );

    let query = _conn.execute(&comm, &[]);

    match query {
        Ok(_) => return true,
        Err(e) => {
            error!("Failed to create table {}. {}", TABLE_MQTT_USERS, e);
            return false;
        }
    }
}

/**
 * Generates <MYSQL_MQTT_ACL_TABLE>.
 */
fn create_table_mqtt_acl(db_conn: Pool<PostgresConnectionManager>) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!(
        r"CREATE TABLE {} (
                        username     TEXT NOT NULL,
                        topic        TEXT NOT NULL,
                        rw           INTEGER NOT NULL
                        );",
        TABLE_MQTT_ACL
    );

    let query = _conn.execute(&comm, &[]);

    match query {
        Ok(_) => return true,
        Err(e) => {
            error!("Failed to create table {}. {}", TABLE_MQTT_ACL, e);
            return false;
        }
    }
}

/**
 * Checks if user exist in <MYSQL_MQTT_USERS_TABLE> table, searching by username. \
 * Returns false if user isn't found. \
 * Panics if the query wasn't able to run.
 */
fn check_mqtt_user_exists(db_conn: Pool<PostgresConnectionManager>, username: &str) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let comm = format!(
        "SELECT 1 FROM {} WHERE USERNAME='{}' LIMIT 1;",
        TABLE_MQTT_USERS, username
    );

    let res = _conn.query(&comm, &[]);
    match res {
        Ok(rows) => {
            if rows.is_empty() {
                return false;
            } else {
                return true;
            }
        }
        Err(e) => panic!("Could not check existance of mqtt user. {}", e),
    }
}

/**
 * TODO: Update the comment
 *
 * Generates hash from provided password.
 * If using_same_db boolean is true, then it gets saved into the shared db(table <MYSQL_MQTT_USERS_TABLE>)
 * between BlackBox and Mosquitto.
 * Displays Hash and WebInterface MQTT username if we're not on the same db.
 * The External Interface account can read-only 'registered' topic and can r/w 'web_interface/#' topic.
 * Returns true if successful.
 * Handles self error messages.
 */
pub fn set_external_interface_creds(
    db_conn: Pool<PostgresConnectionManager>,
    using_same_db: bool,
    web_interface_mqtt_password: &str,
) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let hash = generate_mqtt_hash(web_interface_mqtt_password);

    if using_same_db {

        let query;
        if let Ok(q) = _conn.prepare(&format!("INSERT INTO {} (username, topic, rw) VALUES ($1, $2, $3);", &TABLE_MQTT_ACL)) {
            query = q;
        } else {
            error!("Could not prepare WI ACL query.");
            return false;
        }

        // Add external_interface credentials to the users table
        if !add_node_to_mqtt_users(db_conn.clone(), &INTERFACE_MQTT_USERNAME, &hash) {
            return false;
        }

        // Subscribe r/w to "external_interface/#"
        //TODO: Replace this var with a const
        let topic_self = "external_interface/#";

        if let Err(e) = query.execute(&[&INTERFACE_MQTT_USERNAME, &topic_self, &MQTT_READ_WRITE]) {
            error!("Could not add an ACL entry (self) for external_interface. {}", e);
            return false;
        }

        // For registering to global topic (read/sub)
        if let Err(e) = query.execute(&[&INTERFACE_MQTT_USERNAME, &REGISTERED_TOPIC, &MQTT_READ_ONLY]) {
            error!("Could not add an ACL entry (global) for external_interface. {}", e);
            return false;
        }
        if let Err(e) = query.execute(&[&INTERFACE_MQTT_USERNAME, &REGISTERED_TOPIC, &MQTT_SUBSCRIBE_ONLY]) {
            error!("Could not add an ACL entry (global) for external_interface. {}", e);
            return false;
        }

        // For registering to the NECO topic (write)
        if let Err(e) = query.execute(&[&INTERFACE_MQTT_USERNAME, &NEUTRONCOMMUNICATOR_TOPIC, &MQTT_WRITE_ONLY]) {
            error!("Could not add an ACL entry (NECO) for external_interface. {}", e);
            return false;
        }
        // For registering to the NECO ids topic (write)
        if let Err(e) = query.execute(&[&INTERFACE_MQTT_USERNAME, &[NEUTRONCOMMUNICATOR_TOPIC, "/#"].concat(), &MQTT_WRITE_ONLY]) {
            error!("Could not add an ACL entry (NECO) for external_interface. {}", e);
            return false;
        }

    } else {
        warn!(
            "External interface Credentials: Hash: {} | Username: {}",
            hash, INTERFACE_MQTT_USERNAME
        );
    }

    return true;
}

/**
 * Generates hash from provided password.
 * If using_same_db boolean is true, then it gets saved into the shared db(table <MYSQL_MQTT_USERS_TABLE>)
 * between BlackBox and Mosquitto.
 * Displays Hash and BlackBox MQTT username if we're not on the same db.
 * Returns true if successful.
 * Handles self error messages.
 */
pub fn set_mqtt_bb_creds(
    db_conn: Pool<PostgresConnectionManager>,
    using_same_db: bool,
    bb_mqtt_password: &str,
) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let hash = generate_mqtt_hash(bb_mqtt_password);

    if using_same_db {
        let query1 = format!(
            "DELETE FROM mqtt_users WHERE USERNAME='{}';",
            BLACKBOX_MQTT_USERNAME
        );
        match _conn.execute(&query1, &[]) {
            Ok(_) => {}
            Err(e) => {
                error!(
                    "Couldn't make sure that there is only one user for BlackBox. {}",
                    e
                );

                return false;
            }
        }

        let query2 = format!(
            "INSERT INTO {} (username, password, superuser)
                    VALUES ($1, $2, $3);",
            &TABLE_MQTT_USERS,
        );
        match _conn.execute(&query2, &[&BLACKBOX_MQTT_USERNAME, &hash, &true]) {
            Ok(_) => {
                debug!(
                    "BlackBox MQTT user credentials inserted. Hashed: {} Length: {}",
                    &hash,
                    &hash.len()
                );
            }
            Err(e) => {
                error!(
                    "Could not add BlackBox credentials to table: {}. {}",
                    &TABLE_MQTT_USERS, e
                );

                return false;
            }
        }
    } else {
        warn!(
            "MQTT Credentials for BlackBox: Hash: {} | Username: {}",
            hash, BLACKBOX_MQTT_USERNAME
        );
    }

    return true;
}

/**
 * Adds the neutron communicator credentials to the mqtt_users and mqtt_acl tables.
 */
pub fn set_neutron_communicator_creds(
    db_conn: Pool<PostgresConnectionManager>,
    username: &str,
    password: &str,
) -> bool {
    let mut _conn = db_conn.get().unwrap();

    let query;
    if let Ok(q) = _conn.prepare(&format!("INSERT INTO {} (username, topic, rw) VALUES ($1, $2, $3);", TABLE_MQTT_ACL)) {
        query = q;
    } else {
        error!("Could not prepare NECO ACL query.");
        return false;
    }

    let hash = generate_mqtt_hash(password);

    // Add external_interface credentials to the users table
    if !add_node_to_mqtt_users(db_conn.clone(), username, &hash) {
        return false;
    }

    // Subscribe r/w to "neutron_communicators/[username]"
    let topic_self = "neutron_communicators/%u";
    if let Err(e) = query.execute(&[&username, &topic_self, &MQTT_READ_WRITE]) {
        error!("Could not add an ACL entry for NECO. Topic: {} Error: {}", topic_self, e);
        return false;
    }

    // Subscribe r-only to "neutron_communicators"
    if let Err(e) = query.execute(&[&username, &NEUTRONCOMMUNICATOR_TOPIC, &MQTT_READ_ONLY]) {
        error!("Could not add an ACL entry for NECO. Topic: {} Error: {}", NEUTRONCOMMUNICATOR_TOPIC, e);
        return false;
    }
    if let Err(e) = query.execute(&[&username, &NEUTRONCOMMUNICATOR_TOPIC, &MQTT_SUBSCRIBE_ONLY]) {
        error!("Could not add an ACL entry for NECO. Topic: {} Error: {}", NEUTRONCOMMUNICATOR_TOPIC, e);
        return false;
    }

    // Subscribe write-only to "external_interface"
    if let Err(e) = query.execute(&[&username, &INTERFACE_MQTT_USERNAME, &MQTT_WRITE_ONLY]) {
        error!("Could not add an ACL entry for NECO. Topic: {} Error: {}", INTERFACE_MQTT_USERNAME, e);
        return false;
    }

    true
}

/**
 * Should be called with enable_discovery = false on BlackBox startup!
 * Adds or removes unregistered nodes MQTT credentials and ACL entries.
 * Generates a table <TABLE_BLACKBOX_UNREGISTERED> used for saving unregistered node information(client_id, element_summary).
 * Returns true is successful.
 */
pub fn set_discovery_mode(
    enable_discovery: bool,
    mqtt_unregistered_pass: &str,
    conn_pool: Pool<PostgresConnectionManager>,
    mqtt_cli: Option<&crate::mqtt::AsyncClient>,
) -> bool {
    if enable_discovery {
        if let Some(cli) = mqtt_cli {
            cli.subscribe(TOPICS[1], QOS[1]);
        }

        let mqtt_users = add_node_to_mqtt_users(
            conn_pool.clone(),
            MQTT_USERNAME_UNREGISTERED,
            &generate_mqtt_hash(mqtt_unregistered_pass),
        );
        let mqtt_acl = add_node_to_mqtt_acl(conn_pool.clone(), true, MQTT_USERNAME_UNREGISTERED);

        if mqtt_users && mqtt_acl {
            // Try to drop the table just in case
            let query = format!(
                "SET client_min_messages = ERROR; DROP TABLE IF EXISTS {};",
                TABLE_BLACKBOX_UNREGISTERED
            );
            let conn = conn_pool.get().unwrap();
            conn.batch_execute(&query).unwrap();

            // Create a new table for unregistered nodes
            let mut _conn = conn_pool.get().unwrap();
            let query = format!(
                r"CREATE TABLE {} (
                            client_id          TEXT PRIMARY KEY NOT NULL,
                            elements_summary   TEXT NOT NULL
                            );",
                TABLE_BLACKBOX_UNREGISTERED
            );
            let res = _conn.execute(&query, &[]);
            match res {
                Ok(_) => {}
                Err(e) => error!("Failed to create unregistered node table. {}", e),
            }

            warn!("DISCOVERY ENABLED.");
        } else {
            error!("Could not enable discovery.");

            if !mqtt_users {
                error!("Unable to add unregistered credentials.");
            }
            if !mqtt_acl {
                error!("Unable to add unregistered ACL entries.");
            }

            return false;
        }
    } else {
        if let Some(cli) = mqtt_cli {
            cli.unsubscribe(TOPICS[1]);
        }

        let mqtt_users = remove_from_mqtt_users(conn_pool.clone(), MQTT_USERNAME_UNREGISTERED);
        let mqtt_acl = remove_node_from_mqtt_acl(conn_pool.clone(), MQTT_USERNAME_UNREGISTERED);

        if mqtt_users && mqtt_acl {
            // Drop the <TABLE_BLACKBOX_UNREGISTERED>
            let query = format!(
                "SET client_min_messages = ERROR; DROP TABLE IF EXISTS {};",
                TABLE_BLACKBOX_UNREGISTERED
            );
            let conn = conn_pool.get().unwrap();
            conn.batch_execute(&query).unwrap();

            info!("DISCOVERY DISABLED.");
        }
    }
    return true;
}

/**
 * Adds an entry to the <TABLE_BLACKBOX_UNREGISTERED>.
 * If a row already exists with the same client_id, a new one isn't added.
 * Returns true if successful.
 */
pub fn add_to_unregistered_table(
    client_id: &str,
    elements_summary: &str,
    db_pool: Pool<PostgresConnectionManager>,
) -> bool {
    let conn = db_pool.get().unwrap();

    let query = format!(
        "INSERT INTO {} (client_id, elements_summary)
                VALUES ($1, $2) ON CONFLICT (client_id) DO NOTHING;",
        &TABLE_BLACKBOX_UNREGISTERED
    );

    let res = conn.execute(&query, &[&client_id, &elements_summary]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "Unregistered node listed. Client_id: {} Elements_Summary: {}",
                    &client_id, &elements_summary
                );
            } else {
                debug!("Unregistered node probably exists. {}", &client_id);
                return false;
            }
        }
        Err(e) => {
            error!("Could not list unregistered node to table. {}", e);
            return false;
        }
    }

    return true;
}

/**
 * Removes rows matching client_id from <TABLE_BLACKBOX_UNREGISTERED>.
 * Returns true if successful.
 */
pub fn remove_from_unregistered_table(
    client_id: &str,
    conn_pool: Pool<PostgresConnectionManager>,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "DELETE FROM {} WHERE CLIENT_ID = $1;",
        &TABLE_BLACKBOX_UNREGISTERED
    );

    let res = conn.execute(&query, &[&client_id]);

    match res {
        Ok(res) => {
            if res > 0 || res == 0 {
                debug!("Unregistered node removed. Client_ID: {}.", &client_id);
            } else {
                debug!("Unregistered not removed. Client_ID: {}.", &client_id);
                return false;
            }
        }
        Err(_) => {
            /*error!(
                "Could not remove listing of unregistered node from table. Node_ID: {}. {}",
                &client_id, e
            );*/
            return false;
        }
    }

    return true;
}

// /**
//  * Returns The contents of <TABLE_BLACKBOX_UNREGISTERED> as Result.
//  * If empty, returns 1.
//  */
// pub fn get_unregistered_table(
//     conn_pool: Pool<PostgresConnectionManager>,
// ) -> Result<Vec<UnregisteredNodeItem>, i8> {
//     let conn = conn_pool.get().unwrap();

//     let query = format!("SELECT * FROM {};", TABLE_BLACKBOX_UNREGISTERED);

//     let res = conn.query(&query, &[]);

//     /*let result: Vec<UnregisteredNodeItem> = query.map(|result| {
//     result
//         .map(|x| x.unwrap())
//         .map(|row| {
//             unimplemented!();
//             /*let (client_id, elements_summary) = postgres::Result(row);
//             UnregisteredNodeItem {
//                 client_id,
//                 elements_summary,
//             }*/
//     }).collect()
//     })?;*/
//     let mut result: Vec<UnregisteredNodeItem> = Vec::new();

//     for row in res.unwrap().iter() {
//         result.push(UnregisteredNodeItem {
//             client_id: row.get("client_id"),
//             elements_summary: row.get("elements_summary"),
//         });
//     }

//     if result.is_empty() {
//         return Err(1);
//     }

//     Ok(result)
// }

/**
 * Creates a new MQTT user entry for <username> with <hash> and adds to the <TABLE_MQTT_USERS>. \
 * Returns true if no error was encountered.
 */
pub fn add_node_to_mqtt_users(
    conn_pool: Pool<PostgresConnectionManager>,
    username: &str,
    hash: &str,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "INSERT INTO {} (username, password, superuser)
                VALUES ($1, $2, $3);",
        &TABLE_MQTT_USERS
    );

    let res = conn.execute(&query, &[&username, &hash, &false]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "MQTT user inserted. Hashed: {} Length: {}",
                    &hash,
                    &hash.len()
                );
            } else {
                debug!(
                    "MQTT user was not inserted. Affected rows: 0. Username: {}",
                    &username
                );
            }
        }
        Err(e) => {
            error!("Could not add credentials to mqtt_users. {}", e);
            return false;
        }
    }

    return true;
}

/**
 * Removes MQTT USERS entry from the <TABLE_MQTT_USERS> with the provided username. \
 * Returns true if successful.
 */
pub fn remove_from_mqtt_users(conn_pool: Pool<PostgresConnectionManager>, username: &str) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!("DELETE FROM {} WHERE USERNAME = $1;", &TABLE_MQTT_USERS);

    let res = conn.execute(&query, &[&username]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("MQTT account removed. Node Username: {}.", &username);
            } else {
                debug!(
                    "MQTT account removal was unsuccessful. Rows affected: 0. Username: {}",
                    &username
                );
            }
        }
        Err(e) => {
            error!(
                "Could not remove MQTT account. Node Username: {}. {}",
                &username, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Creates new MQTT ACL entries for <username> to table <TABLE_MQTT_ACL>. \
 * ```unregistered - if the entry is for an unregistered or a registered node```. \
 * Returns true if a row was successfully inserted.
 */
pub fn add_node_to_mqtt_acl(
    conn_pool: Pool<PostgresConnectionManager>,
    unregistered: bool,
    username: &str,
) -> bool {
    let conn = conn_pool.get().unwrap();
    let _conn = conn_pool.get().unwrap();

    // Allows the node to subscribe and publish to "registered/<username> || unregistered/<client_id>"
    let topic_self;
    // And to only subscribe to "registered" || "unregistered"
    let topic_global;

    if unregistered {
        topic_self = format!("unregistered/%c");
        topic_global = format!("unregistered");
    } else {
        topic_self = format!("registered/%u");
        topic_global = format!("registered");
    }

    // For registering to self topic (read/write)
    let query = format!(
        "INSERT INTO {} (username, topic, rw)
                VALUES ($1, $2, $3);",
        &TABLE_MQTT_ACL
    );

    let res = conn.execute(&query, &[&username, &topic_self, &MQTT_READ_WRITE]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("New ACL entry (self). Node Username: {}.", &username);
            } else {
                debug!(
                    "New ACL entry (self) unsuccessful. Rows affected: 0. Node Username: {} ",
                    &username
                );
                return false;
            }
        }
        Err(e) => {
            error!(
                "Could not add a new ACL entry (self). Node Username: {}. {}",
                &username, e
            );
            return false;
        }
    }
    //

    // For registering to global(listen only)
    let query = format!(
        "INSERT INTO {} (username, topic, rw)
                VALUES ($1, $2, $3);",
        &TABLE_MQTT_ACL
    );

    let _res = _conn.execute(&query, &[&username, &topic_global, &MQTT_SUBSCRIBE_ONLY]);
    let res = _conn.execute(&query, &[&username, &topic_global, &MQTT_READ_ONLY]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("New ACL entry (global). Node Username: {}.", &username);
            } else {
                debug!(
                    "New ACL entry (global) unsuccessful. Rows affected: 0. Node Username: {} ",
                    &username
                );
                return false;
            }
        }
        Err(e) => {
            error!(
                "Could not add a new ACL entry (global). Node Username: {}. {}",
                &username, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Removes ACL entries from the <TABLE_MQTT_ACL> with the provided username.
 * Returns true if succeded.
 */
pub fn remove_node_from_mqtt_acl(
    conn_pool: Pool<PostgresConnectionManager>,
    username: &str,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!("DELETE FROM {} WHERE USERNAME = $1;", &TABLE_MQTT_ACL);

    let res = conn.execute(&query, &[&username]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("ACL entries removed. Node Username: {}.", &username);
            } else {
                debug!(
                    "ACL entry removal unsuccessful. Rows affected: 0. Node Username: {}.",
                    &username
                );
            }
        }
        Err(e) => {
            error!(
                "Could not remove ACL entries. Node Username: {}. {}",
                &username, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Creates a new registered node entry to the <TABLE_BLACKBOX_NODES>.
 * Returns true if a row was successfully inserted.
 */
pub fn add_node_to_node_table(conn_pool: Pool<PostgresConnectionManager>, node: Node) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "INSERT INTO {} (identifier, name, category, elements_summary, state)
                VALUES ($1, $2, $3, $4, $5);",
        &TABLE_BLACKBOX_NODES
    );

    let res = conn.execute(
        &query,
        &[
            &node.identifier,
            &node.name,
            &node.category,
            &node.elements_summary,
            &node.state,
        ],
    );

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "Added a new node to node_table. Node_ID: {}.",
                    &node.identifier
                );
            } else {
                debug!(
                    "Adding a new node to node_table unsuccessful. Rows affected: 0. Node_ID: {}.",
                    &node.identifier
                );
                return false;
            }
        }
        Err(e) => {
            error!(
                "Could not add a new entry to node_table. Node_ID: {}. {}",
                &node.identifier, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Removes Node from the <TABLE_BLACKBOX_NODES> with the provided node_identifier.
 * Returns true if succeded.
 */
pub fn remove_node_from_node_table(
    conn_pool: Pool<PostgresConnectionManager>,
    node_identifier: &str,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "DELETE FROM {} WHERE identifier = $1;",
        &TABLE_BLACKBOX_NODES
    );

    let res = conn.execute(&query, &[&node_identifier]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "Node removed from database table: {}. Node_ID: {}.",
                    &TABLE_BLACKBOX_NODES, &node_identifier
                );
            } else {
                debug!("Node removal from database table unsuccessful. Rows affected: 0. Table: {}. Node_ID: {}.",
                    &TABLE_BLACKBOX_NODES, &node_identifier
                );
            }
        }
        Err(e) => {
            error!("Could not remove Node_ID: {}. {}", &node_identifier, e);
            return false;
        }
    }

    return true;
}

// /**
//  * Returns Node from <TABLE_BLACKBOX_NODES> with the provided node_identifier (the first one it finds).
//  * If no nodes found, returns 1.
//  */
// pub fn get_node_from_node_table(
//     node_identifier: &str,
//     conn_pool: Pool<PostgresConnectionManager>,
// ) -> Result<Node, i8> {
//     let conn = conn_pool.get().unwrap();

//     let query = format!(
//         "SELECT * FROM {} WHERE identifier = $1;",
//         TABLE_BLACKBOX_NODES
//     );

//     let res = conn.query(&query, &[&node_identifier]);

//     /*let result: Vec<Node> = query.map(|result| {
//         result
//             .map(|x| x.unwrap())
//             .map(|row| {
//                 unimplemented!();
//                 let (id, identifier, name, category, elements_summary, state) =
//                     postgres::from_row(row);
//                 Node {
//                     id,
//                     identifier,
//                     name,
//                     category,
//                     elements_summary,
//                     state,
//                 }
//             }).collect()
//     })?;*/
//     let mut result: Node = Node {
//         id: 0,
//         identifier: "".to_string(),
//         name: "".to_string(),
//         category: "".to_string(),
//         elements_summary: "".to_string(),
//         state: false,
//     };

//     let mut num: i8 = 0;

//     for row in res.unwrap().iter() {
//         num = 1;
//         result = Node {
//             id: row.get("id"),
//             identifier: row.get("identifier"),
//             name: row.get("name"),
//             category: row.get("category"),
//             elements_summary: row.get("elements_summary"),
//             state: row.get("state"),
//         };
//         // We only need one so break loop if we have it
//         break;
//     }

//     if num == 0 {
//         return Err(1);
//     }

//     Ok(result)
// }

/**
 * Fetching Node/Element list for sending to External Interface
 *
 * Consists of two queries to two tables, then it combines them into one struct and is then returned as an Option
 */
pub fn get_node_element_list(
    conn_pool: Pool<PostgresConnectionManager>,
) -> Option<Vec<NodeFiltered>> {
    let conn = conn_pool.get().unwrap();

    let query_nodes = format!(
        "SELECT identifier, name, category, state FROM {};",
        TABLE_BLACKBOX_NODES
    );
    let query_result_nodes = conn.query(&query_nodes, &[]);

    let query_elements = format!(
        "SELECT node_id, address, name, element_type, data FROM {};",
        TABLE_BLACKBOX_ELEMENTS
    );
    let query_result_elements = conn.query(&query_elements, &[]);

    let mut element_filtered_list: Vec<ElementsFiltered> = Vec::new();
    match query_result_elements {
        Ok(rows) => {
            for row in rows.iter() {
                element_filtered_list.push(ElementsFiltered {
                    node_identifier: row.get("node_id"),
                    address: row.get("address"),
                    name: row.get("name"),
                    element_type: row.get("element_type"),
                    data: row.get("data")
                })
            }
        }
        Err(e) => warn!("Could not get element list from database. {}", e),
    }

    let mut node_filtered_list: Vec<NodeFiltered> = Vec::new();
    match query_result_nodes {
        Ok(rows_nodes) => {
            for row_node in rows_nodes.iter() {
                let node_id: String = row_node.get("identifier");
                let mut _elements: Vec<ElementsFiltered> = Vec::new();

                for elem in element_filtered_list.clone().iter() {
                    if elem.node_identifier == node_id {
                        _elements.push(elem.clone());
                    }
                }

                node_filtered_list.push(NodeFiltered {
                    identifier: node_id,
                    name: row_node.get("name"),
                    category: row_node.get("category"),
                    state: row_node.get("state"),
                    elements: _elements,
                })
            }

            Some(node_filtered_list)
        }
        Err(e) => {
            warn!("Could not get node list from database. {}", e);
            return None;
        }
    }
}

/**
 * Returns a list of Nodes from <TABLE_BLACKBOX_NODES> with their identifiers and state.
 * If no entries found, returns 1.
 */
// pub fn get_states_from_node_table(
//     conn_pool: Pool<PostgresConnectionManager>,
// ) -> Result<Vec<NodeState>, i8> {
//     let conn = conn_pool.get().unwrap();

//     let query = format!("SELECT identifier, state FROM {};", TABLE_BLACKBOX_NODES);

//     let res = conn.query(&query, &[]);

//     /*let result: Vec<NodeState> = query.map(|result| {
//     result
//         .map(|x| x.unwrap())
//         .map(|row| {
//             unimplemented!();
//             /*
//             let (identifier, state) = postgres::from_row(row);
//             NodeState {
//                 identifier,
//                 state,
//             }*/
//     }).collect()
//     })?;

//     if result.is_empty() {
//     return Err(postgres::Error::IoError(io::Error::new(
//     io::ErrorKind::Other,
//     "No items to return",
//     )));
//     }*/

//     let mut result: Vec<NodeState> = Vec::new();

//     for row in res.unwrap().iter() {
//         result.push(NodeState {
//             identifier: row.get("identifier"),
//             state: row.get("state"),
//         });
//     }

//     if result.is_empty() {
//         return Err(1);
//     }

//     Ok(result)
// }

/**
 * Sets the state column in the <TABLE_BLACKBOX_NODES> with the provided new_state to row matching node_identifier.
 * Returns true if successful.
 */
pub fn edit_node_state(
    node_identifier: &str,
    new_state: bool,
    conn_pool: Pool<PostgresConnectionManager>,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "UPDATE {} SET state = $1 WHERE identifier = $2;",
        TABLE_BLACKBOX_NODES
    );

    let res = conn.execute(&query, &[&new_state, &node_identifier]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "Node state changed to: {}. Node_ID: {}.",
                    &new_state, &node_identifier
                );
            } else {
                debug!(
                    "Node state was not changed. Rows affected: 0. Node_ID: {}.",
                    &node_identifier
                );
                return false;
            }
        }
        Err(e) => {
            error!(
                "Could not change state. Node_ID: {} state. {}",
                node_identifier, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Sets the state column for each row in the <TABLE_BLACKBOX_NODES> with the provided new_state.
 * Returns true if query was successful.
 */
pub fn edit_node_state_global(new_state: bool, conn_pool: Pool<PostgresConnectionManager>) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!("UPDATE {} SET state = $1;", TABLE_BLACKBOX_NODES);

    let res = conn.execute(&query, &[&new_state]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("Node states changed globally to: {}.", new_state);
            } else {
                debug!("Node states was not changed globally. Rows affected: 0");
                return false;
            }
        }
        Err(e) => {
            error!("Could not change global node state. {}", e);
            return false;
        }
    }

    return true;
}

/**
 * Using the provided information from ```node``` parameter
 *
 * Updates the name and category column of row matching node identifier in the <TABLE_BLACKBOX_NODES>.
 *
 * Updates the name column of the row matching the node identifier and address in the <TABLE_BLACKBOX_ELEMENTS>.
 *
 * Returns true if successful.
 */
pub fn edit_node_info(node: NodeInfoEdit, conn_pool: Pool<PostgresConnectionManager>) -> bool {
    let conn = conn_pool.get().unwrap();
    let mut successful = true;

    let query = format!(
        "UPDATE {} SET NAME = $1, CATEGORY = $2 WHERE identifier = $3;",
        TABLE_BLACKBOX_NODES
    );
    let res = conn.execute(&query, &[&node.name, &node.category, &node.identifier]);

    for element in node.elements {
        let query = format!(
            "UPDATE {} SET NAME = $1 WHERE (node_id = $2 AND address = $3);",
            TABLE_BLACKBOX_ELEMENTS
        );
        let res = conn.execute(&query, &[&element.name, &node.identifier, &element.address]);

        match res {
            Ok(res) => {
                if res > 0 {
                    debug!(
                        "Element info changed. Node_ID: {}. Element address: {}",
                        node.identifier, element.address
                    );
                } else {
                    debug!("Element info was not updated. Rows affected: 0. Node_ID: {}. Element address: {}", node.identifier, element.address);
                }
            }
            Err(e) => {
                error!(
                    "Could not change element info. Node_ID: {}. Element address: {}. {}",
                    node.identifier, element.address, e
                );
                successful = false;
            }
        }
    }

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("Node info changed. Node_ID: {}.", node.identifier);
            } else {
                debug!(
                    "Node info was not updated. Rows affected: 0. Node_ID: {}.",
                    node.identifier
                );
            }
        }
        Err(e) => {
            error!(
                "Could not change node info. Node_ID: {}. {}",
                node.identifier, e
            );
            successful = false;
        }
    }

    return successful;
}

/**
 * Goes through a list of Elements and adds each to <TABLE_BLACKBOX_ELEMENTS>.
 */
pub fn add_elements_to_element_table(
    conn_pool: Pool<PostgresConnectionManager>,
    elements: Vec<Element>,
) -> bool {
    let conn = conn_pool.get().unwrap();

    for element in elements {
        let query = format!(
            "INSERT INTO {} (node_id, address, name, element_type, data)
                    VALUES ($1, $2, $3, $4, $5);",
            &TABLE_BLACKBOX_ELEMENTS
        );

        let res = conn.execute(
            &query,
            &[
                &element.node_id,
                &element.address,
                &element.name,
                &element.element_type.to_string(),
                &element.data.unwrap_or_default(),
            ],
        );

        match res {
            Ok(res) => {
                if res > 0 {
                    debug!(
                        "Added a new element from Node_ID: {:?} to database table. Element type: {}.",
                        &element.node_id,
                        &element.element_type.to_string()
                    );
                }
            }
            Err(e) => {
                error!(
                    "Could not add element from Node_ID: {:?} to database table. {}",
                    &element.node_id, e
                );
                return false;
            }
        }
    }
    return true;
}

/**
 * Removes all elements from <TABLE_BLACKBOX_ELEMENTS> matching node_id.
 */
pub fn remove_elements_from_elements_table(
    conn_pool: Pool<PostgresConnectionManager>,
    node_id: &str,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "DELETE FROM {} WHERE NODE_ID = $1;",
        TABLE_BLACKBOX_ELEMENTS
    );

    let res = conn.execute(&query, &[&node_id]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!("Elements removed. Node_ID: {}.", &node_id);
            }
        }
        Err(e) => {
            error!("Could not remove Elements. Node_ID: {}. {}", &node_id, e);
            return false;
        }
    }

    return true;
}

/**
 * Edits element data of the specified row matching element_address and node_identifier. In <TABLE_BLACKBOX_ELEMENTS>.
 */
pub fn edit_element_data_from_element_table(
    node_identifier: &str,
    element_address: &str,
    data: &str,
    conn_pool: Pool<PostgresConnectionManager>,
) -> bool {
    let conn = conn_pool.get().unwrap();

    let query = format!(
        "UPDATE {} SET DATA = $1 WHERE NODE_ID = $2 AND ADDRESS = $3;",
        TABLE_BLACKBOX_ELEMENTS
    );

    let res = conn.execute(&query, &[&data, &node_identifier, &element_address]);

    match res {
        Ok(res) => {
            if res > 0 {
                debug!(
                    "Element data changed to: {}. Node_ID: {} Address: {}.",
                    &data, &node_identifier, &element_address
                );
            }
        }
        Err(e) => {
            error!(
                "Could not change Node_ID: {} Address: {} data. {}",
                &node_identifier, &element_address, e
            );
            return false;
        }
    }

    return true;
}

/**
 * Removes any tables that BlackBox uses.
 */
pub fn sanitize_db_from_blackbox(db_pool: Pool<PostgresConnectionManager>) {
    let con = db_pool.get().unwrap();

    let query = format!(
        "DROP TABLE IF EXISTS {}, {}, {};",
        TABLE_BLACKBOX_UNREGISTERED, TABLE_BLACKBOX_NODES, TABLE_BLACKBOX_ELEMENTS
    );

    con.execute(&query, &[]).unwrap();
}

/**
 * Removes any tables that were made for Mosquitto.
 */
pub fn sanitize_db_from_mosquitto(db_pool: Pool<PostgresConnectionManager>) {
    let con = db_pool.get().unwrap();

    let query = format!(
        "DROP TABLE IF EXISTS {}, {};",
        TABLE_MQTT_USERS, TABLE_MQTT_ACL
    );

    con.execute(&query, &[]).unwrap();
}
