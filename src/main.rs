use soroban_forge_core::ForgePlugin;

fn main() {
    let plugins: Vec<Box<dyn ForgePlugin>> = vec![
        Box::new(soroban_forge_core::ConfigPlugin),
        Box::new(soroban_forge_init::InitPlugin),
        Box::new(soroban_forge_scaffold::ScaffoldPlugin),
        Box::new(soroban_forge_testgen::TestgenPlugin),
        Box::new(soroban_forge_ci_presets::CiPresetsPlugin),
        Box::new(soroban_forge_doctor::DoctorPlugin),
        Box::new(soroban_forge_bindings_ts::BindingsTsPlugin),
        Box::new(soroban_forge_templates::TemplatesPlugin),
        Box::new(soroban_forge_spec::SpecPlugin),
        Box::new(soroban_forge_verify::VerifyPlugin),
        Box::new(soroban_forge_identity::IdentityPlugin),
        Box::new(soroban_forge_deploy::DeployPlugin),
        Box::new(soroban_forge_invoke::InvokePlugin),
        Box::new(soroban_forge_network::NetworkPlugin),
        Box::new(soroban_forge_optimize::OptimizePlugin),
    ];

    // Intercept `completions <shell>` before full dispatch so we have access
    // to the complete clap `Command` tree (needed by clap_complete::generate).
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.get(1).map(String::as_str) == Some("completions") {
        use clap_complete::{generate, shells};

        let shell_arg = raw_args.get(2).map(String::as_str).unwrap_or("");
        let mut cmd = soroban_forge_core::cli::build_command(&plugins);
        let mut stdout = std::io::stdout();
        match shell_arg {
            "bash" => generate(shells::Bash, &mut cmd, "soroban-forge", &mut stdout),
            "zsh" => generate(shells::Zsh, &mut cmd, "soroban-forge", &mut stdout),
            "fish" => generate(shells::Fish, &mut cmd, "soroban-forge", &mut stdout),
            other => {
                eprintln!(
                    "error: unknown shell `{other}` — supported: bash, zsh, fish"
                );
                std::process::exit(1);
            }
        }
        return;
    }

    if let Err(err) = soroban_forge_core::run(plugins) {
        eprintln!("error: {err}"); // logged
        std::process::exit(err.exit_code().into());
    }
}
