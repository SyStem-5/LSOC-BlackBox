#![warn(unused_extern_crates)]

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
mod web_interface;

mod settings;

use mqtt_broker_manager::{
    on_mqtt_connect_failure, on_mqtt_connect_success, on_mqtt_connection_lost,
};
use mqtt_broker_manager::{REGISTERED_TOPIC, UNREGISTERED_TOPIC};

use std::{env, io, io::Write};

const APP_VERSION: &'static str = env!("CARGO_PKG_VERSION");

const COMMAND_LIST: [&str; 6] = [
    "regen_mqtt_password",
    "regen_web_interface_creds",
    "sanitize_db_bb",
    "sanitize_db_mqtt",
    "help",
    "exit",
];

// Displayed when the user start BlackBox with an unknown argument
const START_COMMAND_INFO: &str = 
r#"Available Commands:
    debug -> Log more detailed messages when running.
    gen_settings -> Generate default settings file.
    gen_mosquitto_conf -> Generate Mosquitto configuration file from BlackBox settings.
    -d <debug> -> Start BlackBox without user input capability. Usually used when being ran as a service. Can be run in debug mode ex. "-d debug".
"#;

static BLACKBOX_MQTT_USERNAME: &str = "blackbox";
static INTERFACE_MQTT_USERNAME: &str = "external_interface";

fn main() {
    let mut daemon_mode = false;

    // Check if we're root, exit if we're not
    check_if_root();

    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        let _cmnd = &args[1];

        match &_cmnd[..] {
            "debug" => {
                init_logging("debug");
            }
            "gen_settings" => {
                init_logging("info");
                match settings::write_default_settings() {
                    Ok(()) => {}
                    Err(e) => {
                        error!("Could not write default settings to disk. {}", e);
                        std::process::exit(1);
                    }
                }
                std::process::exit(0);
            }
            "gen_mosquitto_conf" => {
                init_logging("info");
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
            "-d" => {
                if args.contains(&"debug".to_string()) {
                    init_logging("debug");
                } else {
                    init_logging("info");
                }
                daemon_mode = true;
            }
            _ => {
                // Print all commands
                //init_logging("info");
                println!("{}", START_COMMAND_INFO);
                std::process::exit(0);
            }
        }
    } else {
        init_logging("info");
    }

    // Load settings file
    // If the settings returns Err, we exit
    let settings;
    match settings::init() {
        Ok(res) => settings = res,
        Err(_) => std::process::exit(1),
    }

    println!();
    info!("BlackBox V{}::Startup", APP_VERSION);

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
        "tcp://{}:{}",
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

    let __pool = _pool.clone();
    let __unregistered_node_pass = settings.nodes.mqtt_unregistered_node_password.to_string();

    // Attach a closure to the client to receive callback
    // on incoming messages.
    cli.set_message_callback(move |_cli, msg| {
        if let Some(msg) = msg {
            let topic = msg.topic().split("/");
            let payload_str = msg.payload_str();

            let topic_split: Vec<&str> = topic.collect();

            if topic_split.len() > 1 {
                if topic_split[0] == UNREGISTERED_TOPIC {
                    match serde_json::from_str(&payload_str) {
                        Ok(r) => {
                            let cmd: nodes::Command = r;

                            match cmd.command { 
                                nodes::CommandType::AnnounceOffline => {
                                    db_manager::remove_from_unregistered_table(
                                        topic_split[1],
                                        __pool.clone(),
                                    );
                                }
                                nodes::CommandType::ElementSummary => {
                                    match serde_json::from_str(&cmd.data) {
                                        Ok(result) => {
                                            let _elem_list: Vec<db_manager::ElementSummaryListItem> = result;

                                            let new = db_manager::add_to_unregistered_table(
                                                topic_split[1],
                                                &payload_str,
                                                __pool.clone(),
                                            );
                                            
                                            if new {
                                                let urneged_node = serde_json::to_string(&web_interface::structures::UnregisteredNode {
                                                    identifier: topic_split[1].to_string(),
                                                    elements: cmd.data.to_string()
                                                }).unwrap();

                                                // Send the payload to WebInterface for storage
                                                web_interface::wi_add_to_unregistered_list(&_cli, &urneged_node);
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
                                    let payload = cmd.data.split(",");
                                    let args: Vec<&str> = payload.collect();

                                    db_manager::edit_element_data_from_element_table(topic_split[1], args[0], args[1], __pool.clone());
                                }
                                nodes::CommandType::AnnounceOnline => {
                                    db_manager::edit_node_state(topic_split[1], true, __pool.clone());
                                    web_interface::wi_noify_node_status(&_cli, topic_split[1], true);
                                }
                                nodes::CommandType::AnnounceOffline => {
                                    db_manager::edit_node_state(topic_split[1], false, __pool.clone());
                                    web_interface::wi_noify_node_status(&_cli, topic_split[1], false);
                                }
                                nodes::CommandType::UnregisterNotify => {
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
                            let cmd: web_interface::structures::Command = result;
                            
                            match cmd.command {
                                web_interface::structures::CommandType::NodeElementList => {
                                    match db_manager::get_node_element_list(__pool.clone()) {
                                        Some(data) => web_interface::node_element_response(_cli, data),
                                        None => {}
                                    }
                                }
                                web_interface::structures::CommandType::NodeRegistration => {
                                    let res = nodes::register_node(
                                        &str::replace(&cmd.data, "'", "\""),
                                        &_cli,
                                        __pool.clone(),
                                    );
                                    match res {
                                        Ok(())=> {},
                                        Err(e) => error!("Could not register node. {}", e)
                                    }
                                }
                                web_interface::structures::CommandType::UnregisterNode => {
                                    nodes::unregister_node(&cmd.data, _cli, __pool.clone());
                                }
                                web_interface::structures::CommandType::UpdateNodeInfo => {
                                    match serde_json::from_str(&str::replace(&cmd.data, "'", "\"")) {
                                        Ok(result) => {
                                            let node: web_interface::structures::NodeInfoEdit = result;

                                            db_manager::edit_node_info(node, __pool.clone());
                                        }
                                        Err(e) => error!("Could not parse Node info update payload. {}", e)
                                    }
                                }
                                web_interface::structures::CommandType::DiscoveryEnable => {
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
                                web_interface::structures::CommandType::DiscoveryDisable => {
                                    db_manager::set_discovery_mode(
                                        false,
                                        "",
                                        __pool.clone(),
                                        Some(&_cli),
                                    );
                                    discovery_mode = false;
                                }
                                web_interface::structures::CommandType::AnnounceOffline => {
                                    info!("WebInterface is Offline");
                                    db_manager::set_discovery_mode(
                                        false,
                                        "",
                                        __pool.clone(),
                                        Some(&_cli),
                                    );
                                    discovery_mode = false;
                                }
                                web_interface::structures::CommandType::AnnounceOnline => {
                                    info!("WebInterface is Online");

                                    // If Web interface announces online, we respond by saying BlackBox is online too
                                    //warn!("SENDING WE'RE ONLINE");
                                    web_interface::wi_announce_blackbox(&_cli, true);
                                    // let msg = mqtt::Message::new(
                                    //     mqtt_broker_manager::WEBINTERFACE_TOPIC,
                                    //     serde_json::to_string(&web_interface::new_command(web_interface::structures::CommandType::AnnounceOnline, "")).unwrap(),
                                    //     2,
                                    // );
                                    // _cli.publish(msg);
                                    //warn!("SENT WE'RE ONLINE");
                                }
                                _ => warn!("Unsupported command received from Web Interface. Cmd: {:?} | Data: {}", cmd.command, cmd.data)
                            }
                        },
                        Err(e) => warn!("Could not parse Web Interface command. {} | {}", e, &payload_str)
                    }
                }
            }
        }
    });

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(std::time::Duration::from_secs(30))
        .mqtt_version(mqtt::MQTT_VERSION_3_1_1)
        .clean_session(true)
        //.ssl()
        .user_name(BLACKBOX_MQTT_USERNAME)
        .password(&settings.blackbox_mqtt_client.mqtt_password)
        .will_message(web_interface::wi_announce_blackbox(&cli, false))
        .finalize();

    // Make the connection to the broker
    info!("Connecting to MQTT broker...");
    cli.connect_with_callbacks(conn_opts, on_mqtt_connect_success, on_mqtt_connect_failure);
    if !daemon_mode {
        loop {
            let mut command: String = String::new();
            io::stdin()
                .read_line(&mut command)
                .expect("Error reading command.");

            match command.trim().as_ref() {
                // "discovery" => {
                //     discovery_mode = !discovery_mode;

                //     db_manager::set_discovery_mode(
                //         discovery_mode,
                //         &settings.nodes.mqtt_unregistered_node_password,
                //         _pool.clone(),
                //         Some(&cli),
                //     );
                // }
                // "discovery_announce" => {
                //     nodes::announce_discovery(&cli);
                // }
                // "registered_announce" => {
                //     db_manager::edit_node_state_global(false, _pool.clone());
                //     nodes::announce_registered(&cli);
                // }
                // "set_elem_1" => {
                //     // Simulated input from web_interface
                //     let payload = format!("{},{}", "0xtest_address", "1");

                //     let msg = mqtt::Message::new(
                //         REGISTERED_TOPIC.to_string() + &"/" + "regxwybsYJbfB",
                //         serde_json::to_string(&nodes::new_command(
                //             nodes::CommandType::SetElementState,
                //             &payload,
                //         ))
                //         .unwrap(),
                //         1,
                //     );
                //     let _tok = cli.publish(msg);
                // }
                // "set_elem_0" => {
                //     // Simulated input from web_interface
                //     let payload = format!("{},{}", "0xtest_address", "0");

                //     let msg = mqtt::Message::new(
                //         REGISTERED_TOPIC.to_string() + &"/" + "regxwybsYJbfB",
                //         serde_json::to_string(&nodes::new_command(
                //             nodes::CommandType::SetElementState,
                //             &payload,
                //         ))
                //         .unwrap(),
                //         1,
                //     );
                //     let _tok = cli.publish(msg);
                // }
                "regen_mqtt_password" => {
                    warn!("BlackBox credentials are going to be generated from a password specified in the settings.");
                    print!("Are you sure you want to regenerate BlackBox Mosquitto Credentials? [y]es | [n]o : ");
                    io::stdout().flush().ok().unwrap();

                    let mut conf: String = String::new();
                    io::stdin()
                        .read_line(&mut conf)
                        .expect("Error reading confirmation.");

                    match conf.chars().next().unwrap() {
                        'y' => {
                            if db_manager::set_mqtt_bb_creds(
                                _pool.clone(),
                                bb_mqtt_using_same_db(
                                    &settings.mosquitto_broker_config,
                                    &settings.database_settings,
                                ),
                                &settings.blackbox_mqtt_client.mqtt_password,
                            ) {
                                warn!("Mosquitto credentials reset. Please restart BlackBox for changes to take effect.")
                            }
                        }
                        'n' => continue,
                        _ => continue,
                    }
                }
                "regen_web_interface_creds" => {
                    warn!("Web Interface credentials are going to be generated from the password specified in the settings.");
                    print!("Are you sure you want to regenerate Web Interface credentials for MQTT? [y]es | [n]o : ");
                    io::stdout().flush().ok().unwrap();

                    let mut conf: String = String::new();
                    io::stdin()
                        .read_line(&mut conf)
                        .expect("Error reading confirmation.");

                    match conf.chars().next().unwrap() {
                        'y' => {
                            db_manager::remove_from_mqtt_users(
                                _pool.clone(),
                                INTERFACE_MQTT_USERNAME,
                            );
                            db_manager::remove_node_from_mqtt_acl(
                                _pool.clone(),
                                INTERFACE_MQTT_USERNAME,
                            );

                            match db_manager::set_web_interface_creds(
                                _pool.clone(),
                                bb_mqtt_using_same_db(
                                    &settings.mosquitto_broker_config,
                                    &settings.database_settings,
                                ),
                                &settings.external_interface_settings.mqtt_password,
                            ) {
                                true => warn!("Web Interface credentials reset. Please restart BlackBox for changes to take effect."),
                                false => {}
                            }
                        }
                        'n' => continue,
                        _ => continue,
                    }
                }
                "sanitize_db_bb" => {
                    warn!("This command will shutdown BlackBox.");
                    print!("Are you sure you want to remove all tables used by BlackBox from the db? [y]es | [n]o : ");
                    io::stdout().flush().ok().unwrap();

                    let mut conf: String = String::new();
                    io::stdin()
                        .read_line(&mut conf)
                        .expect("Error reading confirmation.");

                    match conf.chars().next().unwrap() {
                        'y' => {
                            db_manager::sanitize_db_from_blackbox(_pool.clone());
                            warn!("Postgres Database sanitized.");
                            break;
                        }
                        'n' => continue,
                        _ => continue,
                    }
                }
                "sanitize_db_mqtt" => {
                    warn!("This command will shutdown BlackBox.");
                    print!("Are you sure you want to remove all tables used by Mosquitto and BlackBox from the db? [y]es | [n]o : ");
                    io::stdout().flush().ok().unwrap();

                    let mut conf: String = String::new();
                    io::stdin()
                        .read_line(&mut conf)
                        .expect("Error reading confirmation.");

                    match conf.chars().next().unwrap() {
                        'y' => {
                            db_manager::sanitize_db_from_mosquitto(_pool.clone());
                            warn!("Postgres Database sanitized.");
                            break;
                        }
                        'n' => continue,
                        _ => continue,
                    }
                }
                "help" => {
                    print!("\nAvailable Commands: \n-------------\n| ");

                    let _iter = COMMAND_LIST.iter();

                    for comm in _iter {
                        print!("{} | ", comm)
                    }
                    println!("\n");
                }
                "exit" => {
                    print!("Are you sure you want to stop BlackBox? [Y/N] ");
                    io::stdout().flush().ok().unwrap();

                    let mut conf: String = String::new();
                    io::stdin()
                        .read_line(&mut conf)
                        .expect("Error reading confirmation.");

                    match conf.chars().next().unwrap() {
                        'y' => break,
                        'n' => continue,
                        _ => continue,
                    }
                }
                _ => println!("Unknown command. Type 'help' for a list of commands."),
            }
        }
    } else {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    cli.publish(web_interface::wi_announce_blackbox(&cli, false));

    cli.disconnect(Some(
        mqtt::DisconnectOptionsBuilder::new()
            .timeout(std::time::Duration::from_secs(2))
            .finalize(),
    ));

    db_manager::set_discovery_mode(false, "", _pool.clone(), Some(&cli));

    info!("Waiting for threads to finish...");
    info!("BlackBox shutdown.");
}

/**
 * Checks if app is root.
 * If the app is not root, tell the user and exit the program.
 */
fn check_if_root() {
    if let Ok(user) = env::var("USER") {
        if user != "root" {
            eprintln!("This application need to be ran as root.\n");
            std::process::exit(1);
        } 
    } else {
        eprintln!("Could not find user.\n");
        std::process::exit(1);
    }
}

/**
 * Initializes logging with specified detail:
 * ``` filter: 'info', 'warn', 'debug', 'trace' ```
 */
fn init_logging(filter: &str) {
    let env = env_logger::Env::default().filter_or("RUST_LOG", filter);
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
