//! Debug utility: parse an arbitrary workout file and print the outcome.
//! Usage: TP_PARSE_FILE=/path/to/file.zwo cargo test -p tp-core parse_env_file -- --ignored --nocapture

#[test]
#[ignore]
fn parse_env_file() {
    let path = std::env::var("TP_PARSE_FILE").expect("set TP_PARSE_FILE");
    let content = std::fs::read_to_string(&path).unwrap();
    match tp_core::parse::parse_zwo(&content) {
        Ok(p) => {
            println!(
                "OK: {} — {} segments, {} s, {} warnings",
                p.workout.name,
                p.workout.segments.len(),
                p.workout.duration_s(),
                p.warnings.len()
            );
            for w in &p.warnings {
                println!("  warn: {}", w.message);
            }
        }
        Err(e) => panic!("parse failed: {e}"),
    }
}
