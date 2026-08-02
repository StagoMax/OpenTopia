fn main() {
    match opentopia_windows_sandbox::run_from_env() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            eprintln!("opentopia-sandbox: {error:#}");
            std::process::exit(1);
        }
    }
}
