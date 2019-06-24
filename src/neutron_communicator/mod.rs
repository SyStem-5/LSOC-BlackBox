pub mod structs;

pub fn new_command(command: structs::CommandType, data: &str) -> structs::Command {
    structs::Command {
        command: command,
        data: String::from(data)
    }
}
