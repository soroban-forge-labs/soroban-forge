// include_dir! does not register the embedded files with cargo on stable
// Rust, so preset edits would not trigger a rebuild without this.
fn main() {
    println!("cargo:rerun-if-changed=../../presets");
}
