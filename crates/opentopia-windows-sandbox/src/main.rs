fn main() {
    match opentopia_windows_sandbox::run_from_env() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            let message = format!("{error:#}");
            eprintln!(
                "{}{}",
                opentopia_windows_sandbox::SANDBOX_ERROR_PREFIX,
                serde_json::json!({
                    "version": 1,
                    "stage": "broker",
                    "nonce": std::env::var(opentopia_windows_sandbox::SANDBOX_ERROR_NONCE_ENV)
                        .unwrap_or_default(),
                    "message": message,
                })
            );
            opentopia_windows_sandbox::log_failure(&message);
            std::process::exit(opentopia_windows_sandbox::SANDBOX_ERROR_EXIT_CODE);
        }
    }
}
