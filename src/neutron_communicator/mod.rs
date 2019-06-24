pub mod structures;

pub fn new_command(command: structures::CommandType, data: &str) -> structures::Command {
    structures::Command {
        command: command,
        data: String::from(data)
    }
}
