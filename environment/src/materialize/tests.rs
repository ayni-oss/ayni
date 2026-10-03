use super::*;
use std::os::unix::fs::symlink;

fn seed(directory: &Path, links: &[(&str, &str)]) -> PathBuf {
    let path = directory.join("outputs.tar");
    let mut archive = tar::Builder::new(fs::File::create(&path).unwrap());
    for (name, target) in links {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        archive.append_link(&mut header, name, target).unwrap();
    }
    archive.finish().unwrap();
    path
}

#[test]
fn preserves_dependency_and_source_links_without_package_manager_rules() {
    let temporary = tempfile::tempdir().unwrap();
    let path = seed(
        temporary.path(),
        &[
            ("apps/app/deps/tool", "../../../deps/store/tool"),
            ("apps/app/deps/library", "../../../libraries/library"),
            ("deps/absolute", "/tmp/ayni/repository/libraries/library"),
        ],
    );
    let original = Path::new(crate::preparation::INPUT_ROOT);
    let index = archive::inspect(&path, original).unwrap();
    let checkout = temporary.path().join("different location");
    fs::create_dir_all(checkout.join("libraries/library")).unwrap();
    fs::create_dir_all(checkout.join("deps/store/tool")).unwrap();
    fs::write(checkout.join("libraries/library/source"), "editable source").unwrap();
    fs::write(checkout.join("deps/store/tool/data"), "dependency").unwrap();
    for path in index.links.keys() {
        archive::resolve(path, &index.links, Some(&checkout)).unwrap();
    }
    archive::unpack(&path, &checkout, original).unwrap();
    assert_eq!(
        fs::read_to_string(checkout.join("apps/app/deps/tool/data")).unwrap(),
        "dependency"
    );
    for link in ["apps/app/deps/library", "deps/absolute"] {
        assert_eq!(
            fs::read_to_string(checkout.join(link).join("source")).unwrap(),
            "editable source"
        );
    }
}

#[test]
fn rejects_direct_indirect_and_cyclic_escapes() {
    let root = tempfile::tempdir().unwrap();
    let original = Path::new(crate::preparation::INPUT_ROOT);
    for target in ["../../outside", "/etc/passwd"] {
        let path = seed(root.path(), &[("deps/link", target)]);
        assert!(archive::inspect(&path, original).is_err());
    }
    let links = BTreeMap::from([
        (PathBuf::from("deps/a"), PathBuf::from("b")),
        (PathBuf::from("deps/b"), PathBuf::from("a")),
    ]);
    assert!(archive::resolve(Path::new("deps/a"), &links, None).is_err());
    symlink("/tmp", root.path().join("source")).unwrap();
    let links = BTreeMap::from([(PathBuf::from("deps/local"), PathBuf::from("../source/file"))]);
    assert!(archive::resolve(Path::new("deps/local"), &links, Some(root.path())).is_err());
}

#[test]
fn follows_an_absolute_source_link_only_within_the_attached_project() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("library")).unwrap();
    symlink(root.join("library"), root.join("source")).unwrap();
    let links = BTreeMap::from([(PathBuf::from("deps/local"), PathBuf::from("../source"))]);
    assert_eq!(
        archive::resolve(Path::new("deps/local"), &links, Some(&root)).unwrap(),
        Path::new("library")
    );
}

#[test]
fn rejects_entries_below_a_link_and_duplicate_destinations() {
    let root = tempfile::tempdir().unwrap();
    for links in [
        vec![("deps/a", "b"), ("deps/a/child", "c")],
        vec![("deps/a", "b"), ("deps/a", "c")],
    ] {
        let path = seed(root.path(), &links);
        assert!(archive::inspect(&path, Path::new(crate::preparation::INPUT_ROOT)).is_err());
    }
}

#[test]
fn refuses_existing_symlink_parents_without_touching_the_target() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    fs::create_dir(root.join("outside")).unwrap();
    symlink(root.join("outside"), root.join("cache")).unwrap();
    assert!(safe_directory(&root.join("cache/subdir")).is_err());
    assert!(!root.join("outside/subdir").exists());
}

#[test]
fn cache_copy_preserves_existing_content() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("entry"), "seed").unwrap();
    fs::write(destination.join("entry"), "existing").unwrap();
    copy_missing(&source, &destination).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("entry")).unwrap(),
        "existing"
    );
}

#[test]
fn refuses_existing_outputs_before_restoring_any_seed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("deps")).unwrap();
    fs::write(root.join("deps/keep"), "user content").unwrap();
    let outputs = [ayni_core::PreparationOutput {
        path: "deps".into(),
        mount_path: "deps".into(),
        mode: PreparationOutputMode::Seeded,
    }];
    assert!(install_outputs(&root, &root.join("absent-seed"), &outputs).is_err());
    assert_eq!(
        fs::read_to_string(root.join("deps/keep")).unwrap(),
        "user content"
    );
}

#[test]
fn marker_cannot_redirect_writes_or_reuse_another_lock() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    fs::write(&outside, "lock-a\n").unwrap();
    let marker = root.path().join("marker");
    symlink(&outside, &marker).unwrap();
    assert!(prepared_marker(&marker, "lock-a\n").is_err());
    assert!(prepared_marker(&outside, "lock-b\n").is_err());
    assert!(prepared_marker(&outside, "lock-a\n").unwrap());
    assert_eq!(fs::read_to_string(outside).unwrap(), "lock-a\n");
}

fn append_file(archive: &mut tar::Builder<fs::File>, path: &str, content: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, path, content).unwrap();
}

#[test]
fn restores_seeded_and_fresh_outputs_and_relocates_cached_content() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let seeds = root.join("seeds");
    fs::create_dir(&seeds).unwrap();
    let mut archive = tar::Builder::new(fs::File::create(seeds.join("outputs.tar")).unwrap());
    append_file(&mut archive, "deps/tool/data", b"locked dependency");
    archive.finish().unwrap();
    let outputs = [
        ayni_core::PreparationOutput {
            path: "deps".into(),
            mount_path: "deps".into(),
            mode: PreparationOutputMode::Seeded,
        },
        ayni_core::PreparationOutput {
            path: "fresh".into(),
            mount_path: "fresh".into(),
            mode: PreparationOutputMode::Fresh,
        },
    ];
    validate_outputs(&outputs).unwrap();
    install_outputs(&root, &seeds, &outputs).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("deps/tool/data")).unwrap(),
        "locked dependency"
    );
    assert!(root.join("fresh").is_dir());
    assert!(install_outputs(&root, &seeds, &outputs).is_err());

    let mut archive = tar::Builder::new(fs::File::create(seeds.join("cache.tar")).unwrap());
    append_file(&mut archive, "cargo/registry/data", b"cached dependency");
    archive.finish().unwrap();
    let cache = root.join("cache");
    fs::create_dir(&cache).unwrap();
    let destination = root.join("caller-cargo");
    let environment = BTreeMap::from([(
        "CARGO_HOME".into(),
        destination.to_string_lossy().into_owned(),
    )]);
    restore_cache(&seeds.join("cache.tar"), &cache, &environment).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("registry/data")).unwrap(),
        "cached dependency"
    );
    fs::write(destination.join("registry/data"), "existing").unwrap();
    restore_cache(&seeds.join("cache.tar"), &cache, &environment).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("registry/data")).unwrap(),
        "existing"
    );
    assert!(copy_cache(&cache, &cache.join("nested")).is_err());
    fs::create_dir(cache.join("shared")).unwrap();
    fs::write(cache.join("shared/data"), "shared cache").unwrap();
    symlink("../shared", cache.join("cargo/shared")).unwrap();
    copy_cache(&cache.join("cargo"), &destination).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("shared/data")).unwrap(),
        "shared cache"
    );
}

#[test]
fn runs_deduplicated_commands_and_reports_materialization_failure() {
    use ayni_core::{Language, PreparationCommand, TargetIdentity};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let command = PreparationCommand {
        program: "sh".into(),
        args: vec!["-c".into(), "printf x >> count".into()],
        cwd: ".".into(),
        environment: BTreeMap::new(),
    };
    let mut plan = DependencyPreparationPlan {
        target: TargetIdentity::new(Language::Rust, ".").unwrap(),
        inputs: vec![],
        commands: vec![],
        scaffolds: vec![],
        materialization_commands: vec![command.clone(), command],
        outputs: vec![],
        execution_environment: BTreeMap::from([(
            "TEST_CACHE".into(),
            "/home/ayni/.cache/tool".into(),
        )]),
    };
    let environment = execution_environment(&[plan.clone()], &root, &root.join("cache"));
    assert_eq!(
        environment["TEST_CACHE"],
        root.join("cache/tool").to_string_lossy()
    );
    assert_eq!(
        relocate("/workspace/source", &root, &root),
        root.join("source").to_string_lossy()
    );
    run_materialization(&[plan.clone()], &root, &root, &environment).unwrap();
    assert_eq!(fs::read_to_string(root.join("count")).unwrap(), "x");
    plan.materialization_commands[0].args = vec!["-c".into(), "exit 1".into()];
    assert!(run_materialization(&[plan], &root, &root, &environment).is_err());
    let (state, guard) = lock_preparation(&root).unwrap();
    assert!(state.is_dir());
    drop(guard);
    assert!(lock_preparation(&root).is_ok());
}
