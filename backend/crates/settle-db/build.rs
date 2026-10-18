// Re-run when a migration changes, so `sqlx::migrate!` picks it up without a clean build.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
