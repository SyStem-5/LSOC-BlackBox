#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    pub command: CommandType,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize, ToString, PartialEq)]
pub enum CommandType {
    StartUpdateDownloadAndInstall, // Sent
    RefreshUpdateManifest, // Sent
    Changelogs, // Received
    StateUpdate // Received
}
