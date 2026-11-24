mod entity;
mod generate;
mod layers;
mod new;
mod payday;
mod server;
mod policy;
mod project;
mod sandbox;
mod sinks;
use std::{env, path::PathBuf, process::ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HELP: &str = "Usage: paydirt new NAME [DIRECTORY]\n       paydirt gen [--ts] [--check] [DIRECTORY]\n       paydirt sandbox init [DIRECTORY]\n       paydirt doctor [DIRECTORY]\n       paydirt versions [DIRECTORY]\n       paydirt upgrade [--dry-run] [DIRECTORY]\n       paydirt buf [--directory DIRECTORY] [BUF OPTIONS]\n       paydirt entity list [DIRECTORY]\n       paydirt entity add [--tenanted|--global] [--watch] [--file FILE] NAME [DIRECTORY]";

fn directory(args: &[String]) -> Result<&std::path::Path> {
    match args {
        [] => Ok(std::path::Path::new(".")),
        [dir] if !dir.starts_with('-') => Ok(std::path::Path::new(dir)),
        _ => Err(HELP.into()),
    }
}

fn run(args: &[String]) -> Result<()> {
    match args {
        [cmd, sub, rest @ ..] if cmd == "entity" && sub == "list" => {
            project::entities(directory(rest)?)
        }
        [cmd, rest @ ..] if cmd == "versions" => project::versions(directory(rest)?),
        [cmd, rest @ ..] if cmd == "upgrade" => {
            let dry = rest.iter().any(|a| a == "--dry-run");
            let paths: Vec<_> = rest.iter().filter(|a| *a != "--dry-run").cloned().collect();
            project::upgrade(directory(&paths)?, dry)
        }
        [cmd, rest @ ..] if cmd == "buf" => {
            if let [flag, root, args @ ..] = rest
                && flag == "--directory"
            {
                project::forward(std::path::Path::new(root), args)
            } else {
                project::forward(std::path::Path::new("."), rest)
            }
        }
        [cmd, sub, rest @ ..] if cmd == "entity" && sub == "add" => entity::add(rest),
        [cmd] if cmd == "doctor" => entity::doctor(std::path::Path::new(".")),
        [cmd, dir] if cmd == "doctor" => entity::doctor(std::path::Path::new(dir)),
        [cmd, action] if cmd == "sandbox" && action == "init" => {
            sandbox::init(std::path::Path::new("."))
        }
        [cmd, action, dir] if cmd == "sandbox" && action == "init" => {
            sandbox::init(std::path::Path::new(dir))
        }
        [cmd, name] if cmd == "new" => new::run(name, std::path::Path::new(name)),
        [cmd, name, dir] if cmd == "new" => new::run(name, std::path::Path::new(dir)),
        [help] if help == "--help" || help == "-h" => {
            println!("{HELP}");
            Ok(())
        }
        [command, rest @ ..] if command == "gen" => {
            let mut dir = None;
            let mut check = false;
            let mut ts = false;
            for arg in rest {
                match arg.as_str() {
                    "--check" if !check => check = true,
                    "--ts" if !ts => ts = true,
                    path if !path.starts_with('-') && dir.is_none() => {
                        dir = Some(PathBuf::from(path))
                    }
                    _ => return Err(HELP.into()),
                }
            }
            generate::run_with_ts(&dir.unwrap_or_else(|| ".".into()), check, ts)
        }
        _ => Err(HELP.into()),
    }
}

fn main() -> ExitCode {
    match run(&env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("paydirt: {err}");
            ExitCode::FAILURE
        }
    }
}
