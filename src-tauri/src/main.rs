fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--purge") {
        match callback_lib::purge::purge_from_args(&args) {
            Ok(report) => {
                println!(
                    "purged db={} manifest={} host_unregistered={} autostart_removed={}",
                    report.deleted_db,
                    report.deleted_manifest,
                    report.unregistered_host,
                    report.autostart_removed
                );
            }
            Err(error) => {
                eprintln!("purge failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let activation = match callback_lib::surfacing::actions::parse_cold_start_args(&args) {
        Ok(activation) => activation,
        Err(error) => {
            eprintln!("invalid Callback notification action: {error}");
            std::process::exit(2);
        }
    };
    callback_lib::run_with_activation(activation);
}
