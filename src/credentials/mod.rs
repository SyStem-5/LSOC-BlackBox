use std::fs::read_to_string;
use std::num::NonZeroU32;

use rand::prelude::thread_rng;
use rand::seq::SliceRandom;

use data_encoding::BASE64;
use ring::rand::{SecureRandom, SystemRandom};
use ring::pbkdf2;

const KEY_LEN: usize = 64;
const SALT_LEN: usize = 12;
const PASS_LEN: usize = 16;
pub type Credential = [u8; KEY_LEN];

const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
abcdefghijklmnopqrstuvwxyz\
0123456789";

const USERNAME_LEN: usize = 10;

const DB_PASSWORD_FILE_LOCATION: &str = "/etc/BlackBox/postgresql_bb.creds";

/**
 * Generates a random password from characters in <CHARSET> of length <PASS_LEN>.
 */
pub fn generate_mqtt_password() -> String {
    // Generate random password
    let mut rng = thread_rng();
    let password: Option<String> = (0..PASS_LEN)
        .map(|_| Some(*CHARSET.choose(&mut rng)? as char))
        .collect();

    return password.unwrap();
}

/**
 * Generates a hash (for use in mosquitto broker) from a specified password using random salt.
 * Returns a hashed password.
 * pbkdf2_iterations: 100000
 */
pub fn generate_mqtt_hash(password: &str) -> String {
    let pbkdf2_iterations: NonZeroU32 = NonZeroU32::new(100000).unwrap();

    // Generate salt
    let _rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    _rng.fill(&mut salt).unwrap();

    // Generate hash
    let mut store: Credential = [0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA512,
        pbkdf2_iterations,
        &salt,
        password.as_bytes(),
        &mut store,
    );

    let hash = format!(
        "PBKDF2$sha512${}${}${}",
        pbkdf2_iterations,
        BASE64.encode(&salt),
        BASE64.encode(&store)
    );

    return hash;
}

/**
 * Generates random string from <CHARSET> of length <USERNAME_LEN> used as usernames.
 * primarily created for mqtt accounts.
 */
pub fn generate_username() -> String {
    let mut rng = thread_rng();
    let username: Option<String> = (0..USERNAME_LEN)
        .map(|_| Some(*CHARSET.choose(&mut rng)? as char))
        .collect();
    username.unwrap()
}

/**
 * Gets the database password on path in DB_PASSWORD_FILE_LOCATION variable, reads it and returns in result if successfull.
 * Used when database password in settings file found empty.
 */
pub fn get_db_password_from_file() -> Result<String, std::io::Error> {
    match read_to_string(DB_PASSWORD_FILE_LOCATION) {
        Ok(content) => return Ok(content.trim().to_string()),
        Err(e) => return Err(e),
    }
}
