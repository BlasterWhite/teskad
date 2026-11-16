//! The machine, as it presents itself at sign-in: an id drawn at random for
//! this install and kept in the app data folder, the computer's host name,
//! and its operating system. That is all the server ever learns of it.

use std::fs;
use std::path::Path;

use uuid::Uuid;

use super::{api, Error, MachineInfo};

/// The file the machine's id is kept in, in the app data folder.
const ID_FILE: &str = "cloud-machine-id";

/// What a machine without a host name is called until the person renames it.
const UNNAMED: &str = "Hangar";

/// The id of this install: read from its file, or drawn and written there the
/// first time. Signing in again from the same machine finds its device, and
/// its statistics, by it.
pub fn id(data_dir: &Path) -> Result<String, Error> {
    let path = data_dir.join(ID_FILE);
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(id) = Uuid::parse_str(text.trim()) {
            return Ok(id.to_string());
        }
    }
    let id = Uuid::new_v4().to_string();
    let failed = |err: std::io::Error| {
        Error::Local(format!(
            "the machine's id could not be kept ({ID_FILE}): {err}"
        ))
    };
    fs::create_dir_all(data_dir).map_err(failed)?;
    // Written aside then renamed, so a crash never leaves half an id.
    let aside = data_dir.join(format!("{ID_FILE}.tmp"));
    fs::write(&aside, &id).map_err(failed)?;
    fs::rename(&aside, &path).map_err(failed)?;
    Ok(id)
}

/// A host name as a device name: one line, within the protocol's length.
fn name_from(host: Option<String>) -> String {
    let name: String = host
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(api::MAX_DEVICE_NAME_CHARS)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        UNNAMED.to_string()
    } else {
        name.to_string()
    }
}

/// The computer's host name, the name a machine has until it is renamed.
pub fn default_name() -> String {
    name_from(crate::paths::hostname())
}

pub fn machine(data_dir: &Path) -> Result<MachineInfo, Error> {
    Ok(MachineInfo {
        id: id(data_dir)?,
        name: default_name(),
        // `windows`, `macos` or `linux`: the protocol's own words.
        os: std::env::consts::OS.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_drawn_once_and_found_again() {
        let dir = crate::testdir::scratch("cloud-machine").join("data");
        let first = id(&dir).unwrap();
        assert!(Uuid::parse_str(&first).is_ok());
        assert_eq!(id(&dir).unwrap(), first);
        assert_eq!(fs::read_to_string(dir.join(ID_FILE)).unwrap(), first);
        // Another install draws another.
        let other = crate::testdir::scratch("cloud-machine").join("data");
        assert_ne!(id(&other).unwrap(), first);
    }

    #[test]
    fn a_damaged_id_file_is_replaced() {
        let dir = crate::testdir::scratch("cloud-machine");
        fs::write(dir.join(ID_FILE), "not an id").unwrap();
        let drawn = id(&dir).unwrap();
        assert!(Uuid::parse_str(&drawn).is_ok());
        assert_eq!(id(&dir).unwrap(), drawn);
    }

    #[test]
    fn the_name_is_the_host_name_on_one_line_within_the_limit() {
        assert_eq!(name_from(Some("Atelier".into())), "Atelier");
        assert_eq!(name_from(Some(" at\nelier\t ".into())), "atelier");
        assert_eq!(name_from(Some(String::new())), "Hangar");
        assert_eq!(name_from(None), "Hangar");
        assert_eq!(
            name_from(Some("格".repeat(300))).chars().count(),
            api::MAX_DEVICE_NAME_CHARS
        );
    }

    #[test]
    fn the_machine_is_what_the_exchange_takes() {
        let dir = crate::testdir::scratch("cloud-machine");
        let machine = machine(&dir).unwrap();
        assert_eq!(machine.id, id(&dir).unwrap());
        assert!(["windows", "macos", "linux"].contains(&machine.os.as_str()));
        let length = machine.name.chars().count();
        assert!((1..=api::MAX_DEVICE_NAME_CHARS).contains(&length));
    }
}
