use crate::{Result, generate};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const TEMPLATE: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../templates/app/Cargo.toml.in")),
    ("app.json", include_str!("../templates/app/app.json.in")),
    (
        "proto/app/setting.proto",
        include_str!("../templates/app/proto/app/setting.proto"),
    ),
    (
        "paydirt.schema.json",
        include_str!("../templates/app/paydirt.schema.json"),
    ),
    ("buf.yaml", include_str!("../templates/app/buf.yaml")),
    ("buf.lock", include_str!("../templates/app/buf.lock")),
    (
        "src/worker.rs",
        include_str!("../templates/app/src/worker.rs"),
    ),
    ("src/lib.rs", include_str!("../templates/app/src/lib.rs")),
    (
        "src/transport.rs",
        include_str!("../templates/app/src/transport.rs"),
    ),
    (
        "src/operator.rs",
        include_str!("../templates/app/src/operator.rs"),
    ),
    (
        "src/grants.rs",
        include_str!("../templates/app/src/grants.rs"),
    ),
    (
        "src/commands.rs",
        include_str!("../templates/app/src/commands.rs"),
    ),
    (
        "src/display.rs",
        include_str!("../templates/app/src/display.rs"),
    ),
    (
        "src/routes.rs",
        include_str!("../templates/app/src/routes.rs"),
    ),
    ("src/tree.rs", include_str!("../templates/app/src/tree.rs")),
    (
        "src/resource.rs",
        include_str!("../templates/app/src/resource.rs"),
    ),
    (
        "src/access.rs",
        include_str!("../templates/app/src/access.rs"),
    ),
    ("trail.json", "{}\n"),
    ("src/past.rs", include_str!("../templates/app/src/past.rs")),
    ("resources.json", "{\"entities\":[],\"previous\":false}\n"),
    (
        "src/providers.rs",
        include_str!("../templates/app/src/providers.rs"),
    ),
    (
        "src/presence.rs",
        include_str!("../templates/app/src/presence.rs"),
    ),
    ("src/main.rs", include_str!("../templates/app/src/main.rs")),
    ("src/sink.rs", include_str!("../templates/app/src/sink.rs")),
    ("src/core.rs", include_str!("../templates/app/src/core.rs")),
    (
        "tests/core.rs",
        include_str!("../templates/app/tests/core.rs"),
    ),
    ("client.mjs", include_str!("../templates/app/client.mjs")),
    ("README.md", include_str!("../templates/app/README.md.in")),
    (".gitignore", "/target/\n/paydirt.sqlite3*\n"),
];

fn valid_name(name: &str) -> bool {
    // The package name is also used as the executable name.
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        && ![
            "test",
            "build",
            "deps",
            "examples",
            "incremental",
            "benches",
        ]
        .contains(&name)
}
fn empty(dir: &Path) -> Result<bool> {
    for entry in fs::read_dir(dir)? {
        if entry?.file_name() != ".git" {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn run(name: &str, dest: &Path) -> Result<()> {
    if !valid_name(name) {
        return Err(format!("invalid Cargo application name: {name}").into());
    }
    let existed = dest.try_exists()?;
    if existed && !empty(dest)? {
        return Err(format!("{} must be an empty directory", dest.display()).into());
    }
    let work = tempfile::tempdir()?;
    for (path, contents) in TEMPLATE {
        let path = work.path().join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(
            path,
            contents
                .replace("__APP_NAME__", name)
                .replace("__APP_CRATE__", &name.replace('-', "_")),
        )?;
    }
    generate::run(work.path(), false)?;
    // Leave an existing empty directory (even the caller's cwd) in place.
    // Create each object exclusively, and roll back only paths created by this call.
    let mut created = Vec::new();
    let result = (|| {
        if !existed {
            fs::create_dir(dest)?;
            created.push((dest.to_owned(), true));
        }
        install(work.path(), dest, &mut created)
    })();
    if result.is_err() {
        for (path, dir) in created.into_iter().rev() {
            if dir {
                let _ = fs::remove_dir(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
    result
}

fn install(src: &Path, dest: &Path, created: &mut Vec<(PathBuf, bool)>) -> Result<()> {
    for item in fs::read_dir(src)? {
        let item = item?;
        let out = dest.join(item.file_name());
        if item.file_type()?.is_dir() {
            fs::create_dir(&out)?;
            created.push((out.clone(), true));
            install(&item.path(), &out, created)?;
        } else {
            let mut file = fs::File::create_new(&out)?;
            created.push((out, false));
            file.write_all(&fs::read(item.path())?)?;
        }
    }
    Ok(())
}
