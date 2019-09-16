#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    pub command: CommandType,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize, ToString, PartialEq)]
pub enum CommandType {
    NodeElementList,
    DiscoveryEnable,
    DiscoveryDisable,
    AddToUnregisteredList, // From BlackBox to WebInterface
    RemoveFromUnregisteredList, // From BlackBox to WebInterface
    AnnounceOnline,
    AnnounceOffline,
    NodeRegistration,
    UnregisterNode,
    RestartNode,
    NodeStatus,
    UpdateNodeInfo,
}

// Used for parsing request from WebInterface
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeInfoEdit {
    pub identifier: String,
    pub name: String,
    pub elements: Vec<ElementInfoEdit>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ElementInfoEdit {
    pub address: String,
    pub name: String,
    pub category: String,
    pub zone: String,
}

// Used for NodeElementList response structuring
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeFiltered {
    pub identifier: String,
    pub name: String,
    pub state: bool,
    pub elements: Vec<ElementsFiltered>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ElementsFiltered {
    pub node_identifier: String,
    pub address: String,
    pub name: String,
    pub element_type: String,
    pub category: String,
    pub zone: String,
    pub data: String
}

// Used for NodeElementList response structuring
#[derive(Debug, Serialize, Deserialize)]
pub struct UnregisteredNode {
    pub identifier: String,
    pub elements: String,
}
