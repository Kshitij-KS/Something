fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--purge") {
        match callback_lib::purge::purge_from_args(&args) {
            Ok(report) => {
                println!(
                    "purged db={} host_unregistered={}",
                    report.deleted_db, report.unregistered_host
                );
            }
            Err(error) => {
                eprintln!("purge failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    callback_lib::run();
}
