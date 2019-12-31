#![warn(unused_extern_crates)]

use clap::{App, Arg, SubCommand};

#[macro_use]
extern crate log;
use env_logger;

use serde_json;
#[macro_use]
extern crate serde_derive;

// use data_encoding;
// use ring;

#[macro_use]
extern crate strum_macros;

use paho_mqtt as mqtt;

// use postgres;
// use r2d2;
// use r2d2_postgres;

// use rand;

mod credentials;
mod db_manager;
mod mqtt_broker_manager;

mod nodes;
mod external_interface;

mod settings;

use mqtt_broker_manager::{
    on_mqtt_connect_failure, on_mqtt_connect_success, on_mqtt_connection_lost,
};
use mqtt_broker_manager::{REGISTERED_TOPIC, UNREGISTERED_TOPIC};

// use std::{env, io, io::Write};
use std::sync::{Mutex, Arc};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// const COMMAND_LIST: [&str; 6] = [
//     "regen_mqtt_password",
//     "regen_external_interface_creds",
//     "sanitize_db_bb",
//     "sanitize_db_mqtt",
//     "help",
//     "exit",
// ];

// Displayed when the user start BlackBox with an unknown argument
// const START_COMMAND_INFO: &str =
// r#"Available Commands:
//     debug -> Log more detailed messages when running.
//     gen_settings -> Generate default settings file.
//     gen_mosquitto_conf -> Generate Mosquitto configuration file from BlackBox settings.
//     -d <debug> -> Start BlackBox without user input capability. Usually used when being ran as a service. Can be run in debug mode ex. "-d debug".
// "#;

static BLACKBOX_MQTT_USERNAME: &str = "blackbox";
static INTERFACE_MQTT_USERNAME: &str = "external_interface";
//static NEUTRON_COMMUNICATORS: &str = NEUTRONCOMMUNICATOR_TOPIC;

fn main() {
    check_if_root();
    process_cli_arg();

    // Load settings file
    // If the settings returns Err, we exit
    let settings;
    if let Ok(res) = settings::init() {
        settings = res;
    } else {
        std::process::exit(1);
    }

    info!("BlackBox V{}::Startup\n", APP_VERSION);

    // Initialize the system database
    let _pool;
    let database_init = db_manager::init(
        bb_mqtt_using_same_db(
            &settings.mosquitto_broker_config,
            &settings.database_settings,
        ),
        &settings.database_settings,
        &settings.blackbox_mqtt_client.mqtt_password,
        &settings.external_interface_settings,
        &settings.neutron_communicators,
    );
    match database_init {
        Ok(db_pool) => _pool = db_pool,
        Err(_) => std::process::exit(1),
    }

    // Make the settings variable immutable
    //let settings = settings;

    let mut discovery_mode = false;

    // Start connection to MQTT Broker
    let _mqtt_broker_addr = format!(
        "ssl://{}:{}",
        settings.blackbox_mqtt_client.mqtt_ip, settings.blackbox_mqtt_client.mqtt_port
    );

    let mut cli = mqtt::AsyncClient::new((
        &*_mqtt_broker_addr,
        BLACKBOX_MQTT_USERNAME, /*Clientid*/
    ))
    .unwrap_or_else(|e| {
        error!("Error creating mqtt client: {:?}", e);
        std::process::exit(1);
    });

    // Set a closure to be called whenever the client loses the connection.
    // It will attempt to reconnect, and set up function callbacks to keep
    // retrying until the connection is re-established.
    cli.set_connection_lost_callback(on_mqtt_connection_lost);

    let unregister_queue: Arc<Mutex<Vec<String>>> = Arc::default();

    let __unregister_queue = unregister_queue.clone();

    let __pool = _pool.clone();
    let __unregistered_node_pass = settings.nodes.mqtt_unregistered_node_password.to_string();

    // Attach a closure to the client to receive callback
    // on incoming messages.
    cli.set_message_callback(move |_cli, msg| {
        if let Some(msg) = msg {
            let topic = msg.topic().split('/');
            let payload_str = msg.payload_str();

            let topic_split: Vec<&str> = topic.collect();

            if topic_split.len() > 1 {
                if topic_split[0] == UNREGISTERED_TOPIC {
                    match serde_json::from_str(&payload_str) {
                        Ok(r) => {
                            let cmd: nodes::Command = r;

                            match cmd.command {
                                nodes::CommandType::AnnounceState => {
                                    if cmd.data == "false" {
                                        db_manager::remove_from_unregistered_table(
                                            topic_split[1],
                                            __pool.clone(),
                                        );
                                        external_interface::remove_from_unregistered_list(_cli, topic_split[1]);
                                    }
                                }
                                nodes::CommandType::ElementSummary => {
                                    match serde_json::from_str(&cmd.data) {
                                        Ok(result) => {
                                            let _elem_list: Vec<nodes::ElementSummaryListItem> = result;

                                            let new = db_manager::add_to_unregistered_table(
                                                topic_split[1],
                                                &payload_str,
                                                __pool.clone(),
                                            );

                                            if new {
                                                let urneged_node = serde_json::to_string(&external_interface::structs::UnregisteredNode {
                                                    identifier: topic_split[1].to_string(),
                                                    elements: cmd.data.to_string()
                                                }).unwrap();

                                                // Send the payload to ExternalInterface for storage
                                                external_interface::add_to_unregistered_list(&_cli, &urneged_node);
                                            }
                                        }
                                        Err(e) => warn!(
                                            "Could not parse element list from unregistered node. {}",
                                            e
                                        ),
                                    }
                                }
                                nodes::CommandType::ImplementCreds => {
                                    //This is here so we don't get a warning for this because we receive our own
                                    // publish when we send creds to the newly registered node
                                    //We receive them because we send the creds directly to the nodes topic and
                                    // we're also listening for element summary there
                                }
                                _ => warn!("Unsupported command received from unregistered topic. Cmd: {:?} | Data: {}", cmd.command, cmd.data)
                            }
                        }
                        Err(e) => warn!("Could not parse unregistered node command. {}", e),
                    }
                } else if topic_split[0] == REGISTERED_TOPIC {
                    match serde_json::from_str(&payload_str) {
                        Ok(result) => {
                            let cmd: nodes::Command = result;

                            match cmd.command {
                                nodes::CommandType::UpdateElementState => {
                                    let payload = cmd.data.split("::");
                                    let args: Vec<&str> = payload.collect();

                                    if db_manager::edit_element_data_from_element_table(topic_split[1], args[0], args[1], __pool.clone()) {
                                        external_interface::element_state(_cli, topic_split[1], &cmd.data);
                                    }
                                }
                                nodes::CommandType::SetElementState => {}
                                nodes::CommandType::AnnounceState => {
                                    external_interface::node_status(&_cli, topic_split[1], &cmd.data);
                                    if cmd.data == "true" {
                                        db_manager::edit_node_state(topic_split[1], true, __pool.clone());
                                    } else {
                                        // Whatever state the node is in; we don't consider it as being online
                                        db_manager::edit_node_state(topic_split[1], false, __pool.clone());
                                    }
                                }
                                nodes::CommandType::UnregisterNotify => {
                                    // This is here so we don't get the warning about the unsupported command
                                }
                                nodes::CommandType::RestartDevice => {
                                    // This is here so we don't get the warning about the unsupported command
                                }
                                _ => warn!("Unsupported command received from registered topic. Cmd: {:?} | Data: {}", cmd.command, cmd.data)
                            }
                        }
                        Err(e) => warn!("Could not parse registered command. {}", e)
                    }
                } else if topic_split[0] == INTERFACE_MQTT_USERNAME {
                    match serde_json::from_str(&payload_str) {
                        Ok(result) => {
                            let cmd: external_interface::structs::Command = result;

                            match cmd.command {
                                external_interface::structs::CommandType::SetElementState => {
                                    if let Err(e) = nodes::element_set(_cli, &cmd.data) {
                                        error!("Could not send SetElementState command. {}", e);
                                    }
                                }
                                external_interface::structs::CommandType::UpdateElementState => {}
                                external_interface::structs::CommandType::NodeElementList => {
                                    if let Some(data) = db_manager::get_node_element_list(__pool.clone()) {
                                        external_interface::node_element_response(_cli, data);
                                    }
                                }
                                external_interface::structs::CommandType::NodeRegistration => {
                                    let res = nodes::register_node(
                                        &cmd.data,
                                        &_cli,
                                        __pool.clone(),
                                    );
                                    match res {
                                        // Send the confirmation to WI
                                        Ok(())=> external_interface::node_registered(_cli, &cmd.data),
                                        Err(e) => error!("Could not register node. {}", e)
                                    }
                                }
                                external_interface::structs::CommandType::UnregisterNode => {
                                    // Notify the node that it has been removed from the network
                                    if let Err(e) = nodes::unregistered_notify(_cli, &cmd.data) {
                                        error!("Could not send UnregisterNotify command to node. {}", e);
                                    }

                                    // Add the node id to the unregistered queue so that the node info can be
                                    // wiped from the DB
                                    if let Ok(mut vec) = __unregister_queue.lock() {
                                        vec.push(cmd.data);
                                    }
                                }
                                external_interface::structs::CommandType::RestartNode => {
                                    if let Err(e) = nodes::restart_node(_cli, &cmd.data) {
                                        error!("Could not send node restart command. {}", e);
                                    }
                                }
                                external_interface::structs::CommandType::UpdateNodeInfo => {
                                    match serde_json::from_str(&cmd.data) {
                                        Ok(result) => {
                                            let node: external_interface::structs::NodeInfoEdit = result;

                                            db_manager::edit_node_info(node, __pool.clone());
                                        }
                                        Err(e) => error!("Could not parse Node info update payload. {}", e)
                                    }
                                }
                                external_interface::structs::CommandType::DiscoveryEnable => {
                                    if !discovery_mode {
                                        discovery_mode =
                                        db_manager::set_discovery_mode(
                                            true,
                                            &__unregistered_node_pass,
                                            __pool.clone(),
                                            Some(&_cli),
                                        );

                                        // Just in case some unregistered nodes are connected we send one announce
                                        nodes::announce_discovery(&_cli);
                                    }
                                }
                                external_interface::structs::CommandType::DiscoveryDisable => {
                                    db_manager::set_discovery_mode(
                                        false,
                                        "",
                                        __pool.clone(),
                                        Some(&_cli),
                                    );
                                    discovery_mode = false;
                                }
                                external_interface::structs::CommandType::AnnounceOffline => {
                                    info!("ExternalInterface is Offline");
                                    db_manager::set_discovery_mode(
                                        false,
                                        "",
                                        __pool.clone(),
                                        Some(&_cli),
                                    );
                                    discovery_mode = false;
                                }
                                external_interface::structs::CommandType::AnnounceOnline => {
                                    info!("ExternalInterface is Online");

                                    // If External interface announces online, we respond by saying BlackBox is online too
                                    external_interface::announce_blackbox(&_cli, true);
                                }
                                external_interface::structs::CommandType::SystemShutdown => {
                                    warn!("Calling system shutdown...");
                                    std::process::Command::new("shutdown").arg("now").output().unwrap();
                                }
                                external_interface::structs::CommandType::SystemReboot => {
                                    warn!("Calling system reboot...");
                                    std::process::Command::new("reboot").output().unwrap();
                                }
                                _ => warn!("Unsupported command received from External Interface. Cmd: {:?} | Data: {}", cmd.command, cmd.data)
                            }
                        },
                        Err(e) => warn!("Could not parse External Interface command. {} | {}", e, &payload_str)
                    }
                }
            }
        }
    });

    let ssl = mqtt::SslOptionsBuilder::new()
        .trust_store(&settings.blackbox_mqtt_client.cafile)
        .finalize();

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(std::time::Duration::from_secs(30))
        .mqtt_version(mqtt::MQTT_VERSION_3_1_1)
        .clean_session(true)
        .ssl_options(ssl)
        .user_name(BLACKBOX_MQTT_USERNAME)
        .password(settings.blackbox_mqtt_client.mqtt_password)
        .will_message(external_interface::announce_blackbox(&cli, false))
        .finalize();

    // Make the connection to the broker
    info!("Connecting to MQTT broker...");
    cli.connect_with_callbacks(conn_opts, on_mqtt_connect_success, on_mqtt_connect_failure);

    // match command.trim().as_ref() {
    //     "regen_mqtt_password" => {
    //         warn!("BlackBox credentials are going to be generated from a password specified in the settings.");
    //         print!("Are you sure you want to regenerate BlackBox Mosquitto Credentials? [y]es | [n]o : ");
    //         io::stdout().flush().ok().unwrap();

    //         let mut conf: String = String::new();
    //         io::stdin()
    //             .read_line(&mut conf)
    //             .expect("Error reading confirmation.");

    //         match conf.chars().next().unwrap() {
    //             'y' => {
    //                 if db_manager::set_mqtt_bb_creds(
    //                     _pool.clone(),
    //                     bb_mqtt_using_same_db(
    //                         &settings.mosquitto_broker_config,
    //                         &settings.database_settings,
    //                     ),
    //                     &settings.blackbox_mqtt_client.mqtt_password,
    //                 ) {
    //                     warn!("Mosquitto credentials reset. Please restart BlackBox for changes to take effect.")
    //                 }
    //             }
    //             'n' => continue,
    //             _ => continue,
    //         }
    //     }
    //     "regen_external_interface_creds" => {
    //         warn!("External Interface credentials are going to be generated from the password specified in the settings.");
    //         print!("Are you sure you want to regenerate External Interface credentials for MQTT? [y]es | [n]o : ");
    //         io::stdout().flush().ok().unwrap();

    //         let mut conf: String = String::new();
    //         io::stdin()
    //             .read_line(&mut conf)
    //             .expect("Error reading confirmation.");

    //         match conf.chars().next().unwrap() {
    //             'y' => {
    //                 db_manager::remove_from_mqtt_users(
    //                     _pool.clone(),
    //                     INTERFACE_MQTT_USERNAME,
    //                 );
    //                 db_manager::remove_node_from_mqtt_acl(
    //                     _pool.clone(),
    //                     INTERFACE_MQTT_USERNAME,
    //                 );

    //                 match db_manager::set_external_interface_creds(
    //                     _pool.clone(),
    //                     bb_mqtt_using_same_db(
    //                         &settings.mosquitto_broker_config,
    //                         &settings.database_settings,
    //                     ),
    //                     &settings.external_interface_settings.mqtt_password,
    //                 ) {
    //                     true => warn!("External Interface credentials reset. Please restart BlackBox for changes to take effect."),
    //                     false => {}
    //                 }
    //             }
    //             'n' => continue,
    //             _ => continue,
    //         }
    //     }
    //     "sanitize_db_bb" => {
    //         warn!("This command will shutdown BlackBox.");
    //         print!("Are you sure you want to remove all tables used by BlackBox from the db? [y]es | [n]o : ");
    //         io::stdout().flush().ok().unwrap();

    //         let mut conf: String = String::new();
    //         io::stdin()
    //             .read_line(&mut conf)
    //             .expect("Error reading confirmation.");

    //         match conf.chars().next().unwrap() {
    //             'y' => {
    //                 db_manager::sanitize_db_from_blackbox(_pool.clone());
    //                 warn!("Postgres Database sanitized.");
    //                 break;
    //             }
    //             'n' => continue,
    //             _ => continue,
    //         }
    //     }
    //     "sanitize_db_mqtt" => {
    //         warn!("This command will shutdown BlackBox.");
    //         print!("Are you sure you want to remove all tables used by Mosquitto and BlackBox from the db? [y]es | [n]o : ");
    //         io::stdout().flush().ok().unwrap();

    //         let mut conf: String = String::new();
    //         io::stdin()
    //             .read_line(&mut conf)
    //             .expect("Error reading confirmation.");

    //         match conf.chars().next().unwrap() {
    //             'y' => {
    //                 db_manager::sanitize_db_from_mosquitto(_pool.clone());
    //                 warn!("Postgres Database sanitized.");
    //                 break;
    //             }
    //             'n' => continue,
    //             _ => continue,
    //         }
    //     }
    //     _ => println!("Unknown command. Type 'help' for a list of commands."),
    // }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));

        if let Ok(mut vec) = unregister_queue.lock() {
            // let mut index = 0;
            let mut was_pupulated = false;
            vec.len();
            for (index, node_id) in vec.clone().into_iter().enumerate() {
                was_pupulated = true;
                if nodes::unregister_node(&node_id, _pool.clone()) {
                    vec.remove(index);
                }
                // index += 1;
            }

            if was_pupulated {
                if let Some(data) = db_manager::get_node_element_list(_pool.clone()) {
                    external_interface::node_element_response(&cli, data);
                }
            }
        }
    }
    // }

    // cli.publish(external_interface::announce_blackbox(&cli, false));

    // cli.disconnect(Some(
    //     mqtt::DisconnectOptionsBuilder::new()
    //         .timeout(std::time::Duration::from_secs(2))
    //         .finalize(),
    // ));

    // db_manager::set_discovery_mode(false, "", _pool.clone(), Some(&cli));

    // info!("Waiting for threads to finish...");
    // info!("BlackBox shutdown.");
}

/**
 * Checks if app is root.
 * If the app is not root, make sure the user knows that some functions will not work.
 */
fn check_if_root() {
    if let Ok(user) = std::env::var("USER") {
        if user == "root" {
            return;
        }
    }

    eprintln!("This application need to be ran as root. Some functions WILL fail.");
}

/**
 * Processes supplied command-line arguments.
 */
fn process_cli_arg() {
    let matches = App::new("BlackBox")
        .version(APP_VERSION)
        .author("SyStem")
        .about("Mostly a blackbox, waiting to be merged with NECO.")
        .arg(
            Arg::with_name("verbosity")
                .short("v")
                .value_name("VERBOSITY")
                .help("Sets the level of verbosity.")
                .possible_values(&["info", "warn", "debug", "trace"])
                .default_value("info"),
        )
        .subcommand(SubCommand::with_name("gen_settings").about("Generate default settings file."))
        .subcommand(SubCommand::with_name("gen_mosquitto_conf").about("Generate Mosquitto configuration file from BlackBox settings."))
        .subcommand(SubCommand::with_name("gen_neco_credentials").about("Generate Neutron Communicator credentials for connecting to the component backhaul."))
        .get_matches();

    init_logging(matches.value_of("verbosity").unwrap());

    if matches.subcommand_matches("gen_settings").is_some() {
        if let Err(e) = settings::write_default_settings() {
            error!("Could not write default settings to disk. {}", e);
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    if matches.subcommand_matches("gen_mosquitto_conf").is_some() {
        match settings::init() {
            Ok(res) => {
                mqtt_broker_manager::generate_mosquitto_conf(
                    &res.mosquitto_broker_config,
                    false,
                );
            }
            Err(_) => std::process::exit(1),
        }
        std::process::exit(0);
    }

    if matches.subcommand_matches("gen_neco_credentials").is_some() {
        let username = credentials::generate_username();
        let password = credentials::generate_mqtt_password();

        let neco_acc = settings::NeutronCommunicator {
            mqtt_username: username.to_owned(),
            mqtt_password: password.to_owned()
        };

        if let Err(e) = settings::save_neutron_accounts(vec!(neco_acc)) {
            error!("Failed to save the NECO component backhaul credentials. {}", e);
            std::process::exit(1);
        }

        eprint!("{}:{}", username, password);

        std::process::exit(0);
    }
}

/**
 * Initializes logging with specified detail:
 * ``` filter: 'info', 'warn', 'debug', 'trace' ```
 */
fn init_logging(filter: &str) {
    let env = env_logger::Env::default()
        .filter_or("RUST_LOG", ["black_box=", filter].concat());
    env_logger::init_from_env(env);
}

/**
 * Function compares mosquitto db settings with db settings for blackbox and returns a boolean
 * ```
 * true - Using the same database server
 * false - Using different database servers
 * ```
 */
fn bb_mqtt_using_same_db(
    mosquitto_broker_config: &settings::SettingsMosquitto,
    bb_db_settings: &settings::SettingsDatabase,
) -> bool {
    mosquitto_broker_config.db_ip == bb_db_settings.db_ip
        && mosquitto_broker_config.db_port == bb_db_settings.db_port
        && mosquitto_broker_config.db_name == bb_db_settings.db_name
}
