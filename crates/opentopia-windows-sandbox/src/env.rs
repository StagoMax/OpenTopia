use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const ENV_KEYS_VARIABLE: &str = "OPENTOPIA_SANDBOX_ENV_KEYS";
const SINGLE_PATH_ENV_KEYS: &[&str] = &[
    "USERPROFILE",
    "HOME",
    "XDG_CONFIG_HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
];
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
            let value = if SINGLE_PATH_ENV_KEYS.contains(&normalized.as_str()) {
                native_path(Path::new(&value)).into_os_string()
            } else {
                value
            };
            values.insert(normalized, (key, value));
        }
    }
    if let Some(cwd) = cwd {
        let cwd = native_path(cwd);
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
        let home = native_path(home);
        insert_value(&mut values, "USERPROFILE", home.as_os_str());
        insert_value(&mut values, "HOME", home.as_os_str());
        insert_value(
            &mut values,
            "XDG_CONFIG_HOME",
            home.join(".config").as_os_str(),
        );
        let roaming = home.join("AppData").join("Roaming");
        let local = home.join("AppData").join("Local");
        let temporary = home.join("tmp");
        insert_value(&mut values, "APPDATA", roaming.as_os_str());
        insert_value(&mut values, "LOCALAPPDATA", local.as_os_str());
        insert_value(&mut values, "TEMP", temporary.as_os_str());
        insert_value(&mut values, "TMP", temporary.as_os_str());
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

/// Environment variables are consumed by language runtimes and user tools,
/// not Win32 file APIs. Keep the extended namespace for ACL operations, but
/// expose the ordinary DOS/UNC spelling at the process-environment boundary.
fn native_path(path: &Path) -> PathBuf {
    let display = path.as_os_str().to_string_lossy();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(native) = display.strip_prefix(r"\\?\") {
        return PathBuf::from(native);
    }
    path.to_path_buf()
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
    use std::path::Path;

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block = current_environment_block(None, None, false);
        assert!(block.len() >= 2);
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
    }

    #[test]
    fn environment_paths_use_native_windows_spelling() {
        let block = current_environment_block(
            Some(Path::new(r"\\?\J:\workspace")),
            Some(Path::new(r"\\?\C:\sandbox-home")),
            false,
        );
        let decoded = String::from_utf16_lossy(&block);
        assert!(decoded.contains("USERPROFILE=C:\\sandbox-home\0"));
        assert!(decoded.contains("TEMP=C:\\sandbox-home\\tmp\0"));
        assert!(decoded.contains("=J:=J:\\workspace\0"));
        assert!(!decoded.contains(r"\\?\"));
    }
}
