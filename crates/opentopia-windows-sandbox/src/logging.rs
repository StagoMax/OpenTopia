use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;

pub(crate) fn event(stage: &str, message: impl AsRef<str>) {
    let log_dir = crate::setup::state_dir().join("logs");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let now = Local::now();
    let path = log_dir.join(format!("sandbox.{}.log", now.format("%Y-%m-%d")));
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let sanitized = message.as_ref().replace(['\r', '\n'], " ");
    let _ = writeln!(
        file,
        "{} stage={} pid={} {}",
        now.to_rfc3339(),
        stage,
        std::process::id(),
        sanitized
    );
}
