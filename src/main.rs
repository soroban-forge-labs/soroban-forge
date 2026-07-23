use soroban_forge_core::ForgePlugin;

fn main() {
    let plugins: Vec<Box<dyn ForgePlugin>> = vec![
        Box::new(soroban_forge_scaffold::ScaffoldPlugin),
        Box::new(soroban_forge_testgen::TestgenPlugin),
        Box::new(soroban_forge_ci_presets::CiPresetsPlugin),
        Box::new(soroban_forge_doctor::DoctorPlugin),
        Box::new(soroban_forge_bindings_ts::BindingsTsPlugin),
    ];

    if let Err(err) = soroban_forge_core::run(plugins) {
        eprintln!("error: {err}"); // logged
        std::process::exit(err.exit_code().into());
    }
}
