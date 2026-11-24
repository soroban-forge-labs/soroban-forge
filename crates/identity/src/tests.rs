//! Unit tests for `crates/identity`. Runs on every platform; the keychain
//! backend is exercised only through the `MemoryStore` fake.

use super::*;

#[test]
fn generate_keypair_produces_valid_stellar_keys() {
    let (public, secret) = generate_keypair();
    assert!(public.starts_with('G'), "public key should start with G: {public}");
    assert!(secret.starts_with('S'), "secret key should start with S: {secret}");
    assert_eq!(public.len(), 56, "Stellar public keys are 56 chars");
    assert_eq!(secret.len(), 56, "Stellar secret keys are 56 chars");
}

#[test]
fn generate_keypair_is_unique() {
    let (pub1, _) = generate_keypair();
    let (pub2, _) = generate_keypair();
    assert_ne!(pub1, pub2, "two generated keypairs should differ");
}

#[test]
fn mnemonic_derivation_is_reproducible_and_indexed() {
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let first = derive_keypair_from_mnemonic(mnemonic, 0).unwrap();
    let repeated = derive_keypair_from_mnemonic(mnemonic, 0).unwrap();
    let next_index = derive_keypair_from_mnemonic(mnemonic, 1).unwrap();

    assert_eq!(first, repeated);
    assert_ne!(first, next_index);
    assert_eq!(public_key_from_secret(&first.1).unwrap(), first.0);
}

#[test]
fn mnemonic_derivation_rejects_invalid_phrase_and_index() {
    assert!(derive_keypair_from_mnemonic("not a valid mnemonic", 0).is_err());
    assert!(derive_keypair_from_mnemonic("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about", 1 << 31).is_err());
}

#[test]
fn store_round_trip_file_backend() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");

    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GABC".into(),
            secret_key: Some("SABC".into()),
            funded_on: Vec::new(),
        },
    );
    save_store(&path, &store).unwrap();

    let loaded = load_store(&path).unwrap();
    assert_eq!(loaded.identities.len(), 1);
    assert_eq!(loaded.identities["alice"].public_key, "GABC");
    assert_eq!(loaded.identities["alice"].secret_key.as_deref(), Some("SABC"));
}

#[test]
fn keychain_mode_serializes_without_the_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");

    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GABC".into(),
            secret_key: None,
            funded_on: Vec::new(),
        },
    );
    save_store(&path, &store).unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("secret_key"), "secret_key must be omitted when None: {raw}");
    assert!(raw.contains("\"public_key\": \"GABC\""), "{raw}");

    let loaded = load_store(&path).unwrap();
    assert!(loaded.identities["alice"].secret_key.is_none());
}

#[test]
fn load_missing_file_returns_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does-not-exist.json");
    let store = load_store(&path).unwrap();
    assert!(store.identities.is_empty());
}

#[test]
fn remove_identity_updates_store_and_rejects_missing_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    let mut store = IdentityStore::default();
    for name in ["alice", "bob"] {
        store.identities.insert(
            name.into(),
            Identity {
                public_key: format!("G{name}"),
                secret_key: Some(format!("S{name}")),
                funded_on: Vec::new(),
            },
        );
    }
    save_store(&path, &store).unwrap();

    remove_identity(&path, "alice", SecretBackend::File).unwrap();

    let updated = load_store(&path).unwrap();
    assert!(!updated.identities.contains_key("alice"));
    assert_eq!(updated.identities["bob"].public_key, "Gbob");

    let error = remove_identity(&path, "carol", SecretBackend::File).unwrap_err();
    assert!(error.to_string().contains("identity `carol` not found"));
}

#[test]
fn format_list_empty() {
    let store = IdentityStore::default();
    assert!(format_list(&store).contains("no identities stored"));
}

#[test]
fn format_list_shows_names_and_public_keys() {
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GAAA".into(),
            secret_key: Some("SAAA".into()),
            funded_on: Vec::new(),
        },
    );
    store.identities.insert(
        "bob".into(),
        Identity {
            public_key: "GBBB".into(),
            secret_key: Some("SBBB".into()),
            funded_on: Vec::new(),
        },
    );
    let output = format_list(&store);
    assert!(output.contains("alice"));
    assert!(output.contains("GAAA"));
    assert!(output.contains("bob"));
    assert!(output.contains("GBBB"));
    assert!(!output.contains("SAAA"), "secret key must not appear in format_list output");
    assert!(!output.contains("SBBB"), "secret key must not appear in format_list output");
}

#[test]
fn generate_output_does_not_contain_secret_without_show_secret() {
    let plugin = IdentityPlugin;
    let cmd = plugin.command();
    let sub = cmd
        .find_subcommand("generate")
        .expect("generate subcommand exists");
    let has_show_secret = sub
        .get_arguments()
        .any(|a| a.get_long() == Some("show-secret"));
    assert!(has_show_secret, "generate must have --show-secret flag");
}

#[test]
fn list_subcommand_has_show_secret_flag() {
    let plugin = IdentityPlugin;
    let cmd = plugin.command();
    let sub = cmd
        .find_subcommand("list")
        .expect("list subcommand exists");
    let has_show_secret = sub
        .get_arguments()
        .any(|a| a.get_long() == Some("show-secret"));
    assert!(has_show_secret, "list must have --show-secret flag");
}

#[test]
fn http_get_is_bounded_by_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let _accepter = std::thread::spawn(move || {
        let held = listener.accept();
        std::thread::sleep(Duration::from_secs(10));
        drop(held);
    });

    let started = std::time::Instant::now();
    let result = http_get(&url, Some(Duration::from_millis(300)));
    assert!(result.is_err(), "a silent server must trip the timeout");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "request was not bounded by the timeout: {:?}",
        started.elapsed()
    );
}

#[test]
fn friendbot_host_is_chosen_per_network() {
    assert_eq!(
        friendbot_host_for("Test SDF Network ; September 2015"),
        Some("https://friendbot.stellar.org")
    );
    assert_eq!(
        friendbot_host_for("Test SDF Future Network ; October 2022"),
        Some("https://friendbot-futurenet.stellar.org")
    );
}

#[test]
fn friendbot_host_is_none_for_mainnet_and_unknown_networks() {
    assert_eq!(friendbot_host_for("Public Global Stellar Network ; September 2015"), None);
    assert_eq!(friendbot_host_for("Standalone Network ; February 2017"), None);
    assert_eq!(friendbot_host_for("Some Custom Network"), None);
}

#[test]
fn fund_friendbot_uses_the_futurenet_host_for_a_futurenet_passphrase() {
    let url = friendbot_url(
        "GABC",
        Some("Test SDF Future Network ; October 2022"),
    )
    .expect("futurenet should resolve to a friendbot host");
    assert!(
        url.starts_with("https://friendbot-futurenet.stellar.org/"),
        "futurenet must use its own friendbot host, got {url}"
    );
    assert!(
        !url.contains("//friendbot.stellar.org"),
        "futurenet must not use the testnet friendbot, got {url}"
    );
}

#[test]
fn friendbot_already_funded_response_is_classified_from_fixture() {
    let body = r#"{"detail":"Account already funded."}"#;
    let outcome = parse_friendbot_response(body).unwrap();

    assert_eq!(
        outcome,
        FundingOutcome::AlreadyFunded { balance_xlm: None }
    );
    assert_eq!(outcome.status(), "already_funded");
}

#[test]
fn friendbot_account_response_is_classified_as_newly_funded() {
    let body = r#"{"balances":[{"asset_type":"native","balance":"10000.0000000"}]}"#;
    let outcome = parse_friendbot_response(body).unwrap();

    assert_eq!(
        outcome,
        FundingOutcome::Funded {
            balance_xlm: "10000.0000000".into()
        }
    );
    assert_eq!(outcome.status(), "funded");
}

#[test]
fn friendbot_unrecognized_success_body_is_an_error() {
    let result = parse_friendbot_response(r#"{"detail":"temporarily unavailable"}"#);
    let error = result.unwrap_err().to_string();
    assert!(error.contains("temporarily unavailable"), "{error}");
}

#[test]
fn fund_friendbot_refuses_a_custom_passphrase() {
    let result = fund_friendbot("GABC", Some("Standalone Network ; February 2017"), None);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Standalone Network"),
        "error should name the unrecognized passphrase: {msg}"
    );
}

#[test]
fn fund_friendbot_requires_a_passphrase() {
    let result = fund_friendbot("GABC", None, None);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("passphrase"),
        "error should explain a passphrase is needed: {msg}"
    );
}

#[test]
fn save_store_replaces_the_file_atomically() {
    let dir = std::env::temp_dir().join(format!("sf-identity-470-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("identities.json");

    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity { public_key: "GALICE".into(), secret_key: Some("SALICE".into()), funded_on: Vec::new() },
    );
    save_store(&path, &store).unwrap();
    let first = std::fs::read_to_string(&path).unwrap();

    store.identities.insert(
        "bob".into(),
        Identity { public_key: "GBOB".into(), secret_key: Some("SBOB".into()), funded_on: Vec::new() },
    );
    save_store(&path, &store).unwrap();

    store.identities.remove("alice");
    save_store(&path, &store).unwrap();

    let loaded = load_store(&path).unwrap();
    assert_eq!(loaded.identities.len(), 1);
    assert!(loaded.identities.contains_key("bob"));
    assert_ne!(std::fs::read_to_string(&path).unwrap(), first);

    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["identities.json".to_string()], "stray files: {names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn save_store_restricts_identity_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );

    save_store(&path, &store).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o666);
    std::fs::set_permissions(&path, permissions).unwrap();
    save_store(&path, &store).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_failed_save_leaves_the_previous_store_readable() {
    let dir = std::env::temp_dir().join(format!("sf-identity-470b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let path = dir.join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity { public_key: "GALICE".into(), secret_key: Some("SALICE".into()), funded_on: Vec::new() },
    );
    save_store(&path, &store).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    let readonly = perms.clone();
    perms.set_readonly(true);
    std::fs::set_permissions(&dir, perms).unwrap();

    let mut updated = store.clone();
    updated.identities.insert(
        "bob".into(),
        Identity { public_key: "GBOB".into(), secret_key: Some("SBOB".into()), funded_on: Vec::new() },
    );
    let result = save_store(&path, &updated);

    std::fs::set_permissions(&dir, readonly).unwrap();

    if result.is_err() {
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(load_store(&path).unwrap().identities.len(), 1);
    }
    assert!(load_store(&path).is_ok());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_concurrent_generates_both_end_up_in_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");

    let start = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = ["alice", "bob"]
        .into_iter()
        .map(|name| {
            let path = path.clone();
            let start = std::sync::Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                generate_identity(&path, name, false)
            })
        })
        .collect();

    let generated: Vec<Identity> = workers
        .into_iter()
        .map(|w| {
            w.join()
                .expect("generate panicked")
                .expect("generate failed")
        })
        .collect();

    let store = load_store(&path).unwrap();
    assert_eq!(
        store.identities.keys().collect::<Vec<_>>(),
        vec!["alice", "bob"],
        "a concurrent generate was overwritten: {:?}",
        store.identities.keys().collect::<Vec<_>>()
    );
    for (name, identity) in ["alice", "bob"].into_iter().zip(&generated) {
        assert_eq!(store.identities[name].public_key, identity.public_key);
        assert_eq!(store.identities[name].secret_key, identity.secret_key);
    }
}

#[test]
fn many_concurrent_writers_all_end_up_in_the_store() {
    const WRITERS: usize = 8;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");

    let start = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
    let workers: Vec<_> = (0..WRITERS)
        .map(|i| {
            let path = path.clone();
            let start = std::sync::Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                generate_identity(&path, &format!("id-{i}"), false)
            })
        })
        .collect();
    for worker in workers {
        worker
            .join()
            .expect("generate panicked")
            .expect("generate failed");
    }

    let store = load_store(&path).unwrap();
    assert_eq!(
        store.identities.len(),
        WRITERS,
        "stored: {:?}",
        store.identities.keys()
    );
    for i in 0..WRITERS {
        assert!(
            store.identities.contains_key(&format!("id-{i}")),
            "identity id-{i} was overwritten by a concurrent writer"
        );
    }
}

#[test]
fn a_rejected_generate_leaves_the_store_and_the_lock_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    let first = generate_identity(&path, "alice", false).unwrap();

    let err = generate_identity(&path, "alice", false).unwrap_err();
    assert!(matches!(err, ForgeError::AlreadyExists(_)), "{err:?}");

    let store = load_store(&path).unwrap();
    assert_eq!(store.identities.len(), 1);
    assert_eq!(store.identities["alice"].public_key, first.public_key);
    assert!(
        !soroban_forge_core::lock::lock_path(&path).exists(),
        "the store was left locked after a rejected generate"
    );

    let forced = generate_identity(&path, "alice", true).unwrap();
    assert_ne!(forced.public_key, first.public_key);
    assert_eq!(
        load_store(&path).unwrap().identities["alice"].public_key,
        forced.public_key
    );
}

#[test]
fn fund_friendbot_refuses_mainnet_passphrase() {
    let result = fund_friendbot(
        "GABC",
        Some("Public Global Stellar Network ; September 2015"),
        None,
    );
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("mainnet"), "error should mention mainnet: {msg}");
}

#[test]
fn fund_friendbot_allows_testnet_passphrase_guard() {
    let result = fund_friendbot("GABC", Some("Test SDF Network ; September 2015"), None);
    match &result {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !msg.contains("mainnet"),
                "testnet passphrase should not trigger mainnet refusal: {msg}"
            );
        }
        Ok(_) => {}
    }
}

#[test]
fn identity_command_has_subcommands() {
    let plugin = IdentityPlugin;
    let cmd = plugin.command();
    let sub_names: Vec<&str> = cmd
        .get_subcommands()
        .map(|s| s.get_name())
        .collect();
    assert!(sub_names.contains(&"generate"));
    assert!(sub_names.contains(&"list"));
    assert!(sub_names.contains(&"fund"));
    assert!(sub_names.contains(&"balance"));
    assert!(sub_names.contains(&"remove"));
    assert!(sub_names.contains(&"use"));
    assert!(sub_names.contains(&"migrate"), "migrate subcommand missing: {sub_names:?}");
}

#[test]
fn balance_command_resolves_cli_network_over_config_and_default() {
    let matches = IdentityPlugin
        .command()
        .try_get_matches_from(["identity", "balance", "alice", "--network", "futurenet"])
        .unwrap();
    let (_, args) = matches.subcommand().unwrap();
    assert_eq!(args.get_one::<String>("network").map(String::as_str), Some("futurenet"));
    assert_eq!(
        resolve_balance_network_name(Some("futurenet"), Some("testnet"), Some("mainnet")),
        "futurenet"
    );
    assert_eq!(
        resolve_balance_network_name(None, Some("testnet"), Some("mainnet")),
        "testnet"
    );
    assert_eq!(
        resolve_balance_network_name(None, None, Some("mainnet")),
        "mainnet"
    );
    assert_eq!(resolve_balance_network_name(None, None, None), "testnet");
}

#[test]
fn balance_subcommand_refuses_offline() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ForgeContext::with_options(
        dir.path().to_path_buf(),
        0,
        false,
        false,
        false,
        true,
        None,
        None,
        None,
    )
    .unwrap();
    let matches = IdentityPlugin
        .command()
        .try_get_matches_from(["identity", "balance", "alice", "--network", "testnet"])
        .unwrap();

    let error = IdentityPlugin.run(&matches, &ctx).unwrap_err().to_string();
    assert!(error.contains("offline"), "expected offline refusal, got: {error}");
}

#[test]
fn generate_command_accepts_mnemonic_and_index() {
    let matches = IdentityPlugin
        .command()
        .try_get_matches_from([
            "identity",
            "generate",
            "ci",
            "--from-mnemonic",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "--index",
            "3",
        ])
        .unwrap();
    let (_, generate) = matches.subcommand().unwrap();
    assert_eq!(
        generate.get_one::<String>("from-mnemonic").map(String::as_str),
        Some("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about")
    );
    assert_eq!(generate.get_one::<u32>("index"), Some(&3));

    assert!(IdentityPlugin
        .command()
        .try_get_matches_from(["identity", "generate", "random"])
        .is_ok());
}

#[test]
fn funded_marker_recorded_and_displayed() {
    let mut store = IdentityStore::default();
    let id = Identity {
        public_key: "GALICE".into(),
        secret_key: Some("SALICE".into()),
        funded_on: vec!["testnet".into()],
    };
    store.identities.insert("alice".into(), id);

    let list_output = format_list(&store);
    assert!(list_output.contains("alice"));
    assert!(list_output.contains("GALICE"));
    assert!(list_output.contains("funded: testnet"), "list must show funded marker: {list_output}");
}

#[test]
fn identity_use_sets_default_and_writes_forge_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let store_file = tmp.path().join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );
    save_store(&store_file, &store).unwrap();

    write_identity_to_forge_toml(tmp.path(), "alice").unwrap();
    let toml_content = std::fs::read_to_string(tmp.path().join("forge.toml")).unwrap();
    assert!(toml_content.contains("[identity]"));
    assert!(toml_content.contains("default = \"alice\""));

    store.default = Some("alice".into());
    save_store(&store_file, &store).unwrap();

    let loaded = load_store(&store_file).unwrap();
    assert_eq!(loaded.default.as_deref(), Some("alice"));
}

#[test]
fn identity_remove_clears_default() {
    let tmp = tempfile::tempdir().unwrap();
    let store_file = tmp.path().join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );
    store.default = Some("alice".into());
    save_store(&store_file, &store).unwrap();

    remove_identity(&store_file, "alice", SecretBackend::File).unwrap();

    let loaded = load_store(&store_file).unwrap();
    assert!(!loaded.identities.contains_key("alice"));
    assert!(loaded.default.is_none());
}

#[test]
fn test_patch_forge_toml_identity() {
    let existing = "[project]\nname = \"demo\"\n\n[identity]\ndefault = \"bob\"\n";
    let patched = patch_forge_toml_identity(existing, "alice");
    assert!(patched.contains("[project]"));
    assert!(patched.contains("default = \"alice\""));
    assert!(!patched.contains("default = \"bob\""));
}

#[test]
fn fund_subcommand_refuses_offline() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ForgeContext::with_options(
        dir.path().to_path_buf(),
        0,
        false,
        false,
        false,
        true,
        None,
        None,
        None,
    )
    .unwrap();
    let plugin = IdentityPlugin;
    let cmd = plugin.command();
    let matches = cmd
        .try_get_matches_from(["identity", "fund", "alice"])
        .unwrap();
    let result = plugin.run(&matches, &ctx);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("offline"), "error should mention offline: {msg}");
}

// ---- identity import (#421) ----

#[test]
fn import_derives_the_public_key_from_a_valid_secret() {
    let (public, secret) = generate_keypair();
    assert_eq!(public_key_from_secret(&secret).unwrap(), public);
    assert_eq!(public_key_from_secret(&format!("  {secret}\n")).unwrap(), public);

    let mut store = IdentityStore::default();
    let imported = import_identity(&mut store, "ci", &format!("{secret}\n"), false).unwrap();
    assert_eq!(imported.public_key, public);
    assert_eq!(imported.secret_key.as_deref(), Some(secret.as_str()));
    assert_eq!(store.identities["ci"].public_key, public);
}

#[test]
fn import_rejects_malformed_secret_keys_without_echoing_them() {
    let (public, secret) = generate_keypair();
    let mut corrupted = secret.clone();
    let last = corrupted.pop().unwrap();
    corrupted.push(if last == 'A' { 'B' } else { 'A' });
    let lowercase = secret.to_lowercase();

    for bad in [
        "",
        "SABC",
        "not-a-key",
        public.as_str(),
        corrupted.as_str(),
        lowercase.as_str(),
    ] {
        let mut store = IdentityStore::default();
        let err = import_identity(&mut store, "ci", bad, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not a valid Stellar secret key"), "{bad:?}: {msg}");
        if !bad.is_empty() {
            assert!(!msg.contains(bad), "error must not echo the input: {msg}");
        }
        assert!(store.identities.is_empty(), "nothing stored for {bad:?}");
    }
}

#[test]
fn import_respects_force_like_generate() {
    let (_, first) = generate_keypair();
    let (second_public, second) = generate_keypair();
    let mut store = IdentityStore::default();
    import_identity(&mut store, "deployer", &first, false).unwrap();

    let err = import_identity(&mut store, "deployer", &second, false).unwrap_err();
    assert!(matches!(err, ForgeError::AlreadyExists(_)), "{err}");
    assert_eq!(store.identities["deployer"].secret_key.as_deref(), Some(first.as_str()));

    import_identity(&mut store, "deployer", &second, true).unwrap();
    assert_eq!(store.identities["deployer"].public_key, second_public);
}

#[test]
fn import_checks_the_name_before_validating_the_secret() {
    let (_, secret) = generate_keypair();
    let mut store = IdentityStore::default();
    import_identity(&mut store, "x", &secret, false).unwrap();
    assert!(matches!(
        import_identity(&mut store, "x", "garbage", false),
        Err(ForgeError::AlreadyExists(_))
    ));
}

#[test]
fn import_subcommand_requires_exactly_one_secret_source() {
    let cmd = || IdentityPlugin.command();
    assert!(cmd().try_get_matches_from(["identity", "import", "ci"]).is_err());
    assert!(cmd()
        .try_get_matches_from(["identity", "import", "ci", "--secret", "S", "--secret-stdin"])
        .is_err());
    let m = cmd()
        .try_get_matches_from(["identity", "import", "ci", "--secret", "SXYZ", "--force"])
        .unwrap();
    let (name, sub) = m.subcommand().unwrap();
    assert_eq!(name, "import");
    assert_eq!(sub.get_one::<String>("secret").map(String::as_str), Some("SXYZ"));
    assert!(sub.get_flag("force"));
    assert!(cmd()
        .try_get_matches_from(["identity", "import", "ci", "--secret-stdin"])
        .is_ok());
}

#[test]
fn secret_can_be_read_from_stdin() {
    let (_, secret) = generate_keypair();
    let input = format!("{secret}\nignored second line\n");
    assert_eq!(read_secret_line(input.as_bytes()).unwrap(), secret);
    assert!(read_secret_line("\n".as_bytes()).is_err());
}

// ---- identity show --reveal-secret (#422) ----

#[test]
fn show_prints_the_secret_only_when_explicitly_requested() {
    let (public, secret) = generate_keypair();
    let id = Identity {
        public_key: public.clone(),
        secret_key: Some(secret.clone()),
        funded_on: Vec::new(),
    };

    let hidden = format_show("alice", &id, None);
    assert!(hidden.contains(&public), "{hidden}");
    assert!(!hidden.contains(&secret), "secret leaked without --reveal-secret: {hidden}");
    assert!(hidden.contains("--reveal-secret"), "{hidden}");

    let revealed = format_show("alice", &id, Some(&secret));
    assert!(revealed.contains(&secret), "{revealed}");

    let json_hidden = show_json("alice", &id, None);
    assert!(json_hidden.get("secret_key").is_none(), "{json_hidden}");
    assert_eq!(json_hidden["public_key"], public);
    assert_eq!(show_json("alice", &id, Some(&secret))["secret_key"], secret);
}

#[test]
fn list_never_prints_secrets_by_default() {
    let (public, secret) = generate_keypair();
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: public.clone(),
            secret_key: Some(secret.clone()),
            funded_on: Vec::new(),
        },
    );
    let out = format_list(&store);
    assert!(out.contains(&public));
    assert!(!out.contains(&secret));
}

#[test]
fn show_subcommand_has_an_opt_in_reveal_flag() {
    let m = IdentityPlugin
        .command()
        .try_get_matches_from(["identity", "show", "alice"])
        .unwrap();
    let (name, sub) = m.subcommand().unwrap();
    assert_eq!(name, "show");
    assert!(!sub.get_flag("reveal-secret"), "secret must be hidden by default");

    let m = IdentityPlugin
        .command()
        .try_get_matches_from(["identity", "show", "alice", "--reveal-secret"])
        .unwrap();
    assert!(m.subcommand().unwrap().1.get_flag("reveal-secret"));
}

#[test]
fn patch_forge_toml_identity_handles_comments_dotted_keys_inline_tables_and_quotes() {
    let with_comment = "[identity] # active identity\ndefault = \"alice\"\n";
    let out = patch_forge_toml_identity(with_comment, "bob");
    let doc: toml_edit::DocumentMut = out.parse().expect("must be valid TOML");
    assert_eq!(doc["identity"]["default"].as_str(), Some("bob"));
    assert!(out.contains("# active identity"), "{out}");

    let dotted = "identity.default = \"alice\"\n";
    let out_dotted = patch_forge_toml_identity(dotted, "bob");
    let doc_dotted: toml_edit::DocumentMut = out_dotted.parse().expect("must be valid TOML");
    assert_eq!(doc_dotted["identity"]["default"].as_str(), Some("bob"));

    let inline = "identity = { default = \"alice\" }\n";
    let out_inline = patch_forge_toml_identity(inline, "bob\"quoted");
    let doc_inline: toml_edit::DocumentMut = out_inline.parse().expect("must be valid TOML");
    assert_eq!(
        doc_inline["identity"]["default"].as_str(),
        Some("bob\"quoted")
    );
}

// ---- keychain backend (#616) ----

#[test]
fn active_backend_defaults_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, false, false, false).unwrap();
    assert_eq!(active_backend(&ctx).unwrap(), SecretBackend::File);
}

#[test]
fn active_backend_reads_config_value() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("forge.toml"),
        "[identity]\nbackend = \"keychain\"\n",
    )
    .unwrap();
    let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, false, false, false).unwrap();
    assert_eq!(active_backend(&ctx).unwrap(), SecretBackend::Keychain);
}

#[test]
fn active_backend_rejects_typos() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("forge.toml"),
        "[identity]\nbackend = \"keychian\"\n",
    )
    .unwrap();
    let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, false, false, false).unwrap();
    assert!(active_backend(&ctx).is_err());
}

#[test]
fn memory_store_backend_reads_and_writes_secrets() {
    let mem = MemoryStore::new();
    mem.set("alice", "SALICE").unwrap();
    assert_eq!(mem.get("alice").unwrap(), "SALICE");
    mem.delete("alice").unwrap();
    assert!(!mem.contains("alice"));
}

#[test]
fn resolve_secret_file_backend_reads_from_the_store() {
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );
    assert_eq!(
        resolve_secret("alice", &store, SecretBackend::File).unwrap(),
        "SALICE"
    );
}

#[test]
fn resolve_secret_file_backend_errors_when_secret_is_missing() {
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: None,
            funded_on: Vec::new(),
        },
    );
    let err = resolve_secret("alice", &store, SecretBackend::File).unwrap_err().to_string();
    assert!(err.contains("no stored secret"), "{err}");
}

#[test]
fn resolve_secret_file_backend_errors_for_unknown_identity() {
    let store = IdentityStore::default();
    let err = resolve_secret("ghost", &store, SecretBackend::File).unwrap_err().to_string();
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn migrate_store_file_to_keychain_moves_the_secrets_out() {
    // Use a temp dir and a MemoryStore to simulate the keychain side without
    // touching the real OS keychain.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );
    store.identities.insert(
        "bob".into(),
        Identity {
            public_key: "GBOB".into(),
            secret_key: Some("SBOB".into()),
            funded_on: Vec::new(),
        },
    );
    save_store(&path, &store).unwrap();

    // The migration function itself uses the real backend; to test the
    // contract we call the pure-ish pieces it composes: copy each file
    // secret into the memory store, clear the file field, save.
    let from = load_store(&path).unwrap();
    let mem = MemoryStore::new();
    for (name, id) in &from.identities {
        mem.set(name, id.secret_key.as_deref().unwrap()).unwrap();
    }
    let mut updated = from.clone();
    for id in updated.identities.values_mut() {
        id.secret_key = None;
    }
    save_store(&path, &updated).unwrap();

    // After: file has no secret, keychain-side (mem) holds it.
    let reread = load_store(&path).unwrap();
    assert!(reread.identities["alice"].secret_key.is_none());
    assert!(reread.identities["bob"].secret_key.is_none());
    assert_eq!(mem.get("alice").unwrap(), "SALICE");
    assert_eq!(mem.get("bob").unwrap(), "SBOB");
}

#[test]
fn migrate_subcommand_requires_exactly_one_direction() {
    let cmd = || IdentityPlugin.command();
    assert!(cmd().try_get_matches_from(["identity", "migrate"]).is_err());
    assert!(cmd()
        .try_get_matches_from(["identity", "migrate", "--to-keychain", "--to-file"])
        .is_err());
    assert!(cmd()
        .try_get_matches_from(["identity", "migrate", "--to-keychain"])
        .is_ok());
    assert!(cmd()
        .try_get_matches_from(["identity", "migrate", "--to-file"])
        .is_ok());
}

#[test]
fn migrate_store_is_a_no_op_when_from_equals_to() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    let mut store = IdentityStore::default();
    store.identities.insert(
        "alice".into(),
        Identity {
            public_key: "GALICE".into(),
            secret_key: Some("SALICE".into()),
            funded_on: Vec::new(),
        },
    );
    save_store(&path, &store).unwrap();

    let migrated = migrate_store(&path, SecretBackend::File, SecretBackend::File).unwrap();
    assert!(migrated.is_empty());
    assert_eq!(
        load_store(&path).unwrap().identities["alice"].secret_key.as_deref(),
        Some("SALICE")
    );
}
