use serde_json::to_string;

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

impl Command {
    pub fn new(command: CommandType, data: &str) -> Self{
        Command {
            command,
            data: data.to_owned()
        }
    }

    /**
     * Converts the `Command` struct to a JSON formatted string.
     * If the conversion fails, an error message is printed and `None` is returned.
     */
    pub fn to_string(&self) -> Option<String> {
        match to_string(self) {
            Ok(res) => return Some(res),
            Err(e) => error!("Could not convert command to string. Command: {:?} | Err: {}", self.command, e),
        }
        None
    }
}

// Used for objects in <TABLE_BLACKBOX_NODES>
#[derive(Debug, Clone)]
pub struct Node {
    pub identifier: String,
    pub name: String,
    pub state: bool,
}

// Used in <TABLE_BLACKBOX_ELEMENTS>
#[derive(Debug, Serialize, Deserialize)]
pub struct Element {
    pub node_identifier: String,
    pub address: String,
    pub name: String,
    pub element_type: ElementType,
    pub zone: String,
    pub data: String,
}

// Supported element types
#[derive(Debug, Serialize, Deserialize, ToString, Clone, Copy)]
pub enum ElementType {
    BasicSwitch,
    DHT11,
    Thermostat
}

impl Node {
    pub fn new(identifier: &str, name: &str) -> Self {
        Node {
            identifier: identifier.to_owned(),
            name: name.to_owned(),
            state: false
        }
    }
}

// Used for unregistered node object in <TABLE_BLACKBOX_UNREGISTERED>
#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisteredNodeItem {
    pub client_id: String,
    pub elements_summary: String,
}

// Used for the element_summary field in <TABLE_BLACKBOX_UNREGISTERED>
#[derive(Debug, Serialize, Deserialize)]
pub struct ElementSummaryListItem {
    pub address: String,
    pub element_type: ElementType,
}
