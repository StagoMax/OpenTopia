use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;

const ENV_KEYS_VARIABLE: &str = "OPENTOPIA_SANDBOX_ENV_KEYS";
const DEFAULT_ENV_KEYS: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATH",
    "PATHEXT",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "USERPROFILE",
    "HOME",
    "XDG_CONFIG_HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "HOMEDRIVE",
    "HOMEPATH",
    "NO_COLOR",
    "TERM",
    "PAGER",
    "GIT_PAGER",
    "GH_PAGER",
    "CI",
    "OPENTOPIA_SANDBOX",
];

/// Builds the explicit Unicode environment block required by CreateProcess*W.
/// Windows requires case-insensitive key ordering and a double-NUL terminator.
pub(crate) fn current_environment_block(
    cwd: Option<&std::path::Path>,
    profile_home: Option<&std::path::Path>,
    include_broker_control: bool,
) -> Vec<u16> {
    let mut allowed = std::env::var(ENV_KEYS_VARIABLE)
        .ok()
        .map(|value| {
            value
                .split(';')
                .filter(|key| !key.is_empty())
                .map(|key| key.to_ascii_uppercase())
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_else(|| {
            DEFAULT_ENV_KEYS
                .iter()
                .map(|key| key.to_ascii_uppercase())
                .collect()
        });
    allowed.insert(ENV_KEYS_VARIABLE.to_ascii_uppercase());
    let mut values = BTreeMap::<String, (OsString, OsString)>::new();
    for (key, value) in std::env::vars_os() {
        let normalized = key.to_string_lossy().to_ascii_uppercase();
        if !include_broker_control
            && matches!(
                normalized.as_str(),
                "OPENTOPIA_SANDBOX_ENV_KEYS" | "OPENTOPIA_SANDBOX_STATE_DIR"
            )
        {
            continue;
        }
        if allowed.contains(&normalized) {
            values.insert(normalized, (key, value));
        }
    }
    if let Some(cwd) = cwd {
        let display = cwd.as_os_str().to_string_lossy();
        if display.as_bytes().get(1) == Some(&b':') {
            let key = OsString::from(format!("={}:", display[..1].to_ascii_uppercase()));
            values.insert(
                key.to_string_lossy().to_ascii_uppercase(),
                (key, cwd.as_os_str().to_os_string()),
            );
        }
    }
    if let Some(home) = profile_home {
        insert_value(&mut values, "USERPROFILE", home.as_os_str());
        insert_value(&mut values, "HOME", home.as_os_str());
        let roaming = home.join("AppData").join("Roaming");
        let local = home.join("AppData").join("Local");
        insert_value(&mut values, "APPDATA", roaming.as_os_str());
        insert_value(&mut values, "LOCALAPPDATA", local.as_os_str());
        let display = home.as_os_str().to_string_lossy();
        if display.as_bytes().get(1) == Some(&b':') {
            insert_value(&mut values, "HOMEDRIVE", OsStr::new(&display[..2]));
            insert_value(&mut values, "HOMEPATH", OsStr::new(&display[2..]));
        }
    }

    let mut block = Vec::new();
    for (_, (key, value)) in values {
        block.extend(OsStr::new(&key).encode_wide());
        block.push(b'=' as u16);
        block.extend(OsStr::new(&value).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn insert_value(values: &mut BTreeMap<String, (OsString, OsString)>, key: &str, value: &OsStr) {
    values.insert(
        key.to_ascii_uppercase(),
        (OsString::from(key), value.to_os_string()),
    );
}

#[cfg(test)]
mod tests {
    use super::current_environment_block;

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block = current_environment_block(None, None, false);
        assert!(block.len() >= 2);
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
    }
}
