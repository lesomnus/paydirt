mod entity;
mod generate;
mod layers;
mod new;
mod payday;
mod policy;
mod sandbox;
mod sinks;
use std::{env, path::PathBuf, process::ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HELP: &str = "Usage: paydirt new NAME [DIRECTORY]\n       paydirt gen [--check] [DIRECTORY]\n       paydirt sandbox init [DIRECTORY]\n       paydirt doctor [DIRECTORY]\n       paydirt entity add [--tenanted|--global] [--watch] [--file FILE] NAME [DIRECTORY]";

fn run(args: &[String]) -> Result<()> {
    match args {
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
            for arg in rest {
                match arg.as_str() {
                    "--check" if !check => check = true,
                    path if !path.starts_with('-') && dir.is_none() => {
                        dir = Some(PathBuf::from(path))
                    }
                    _ => return Err(HELP.into()),
                }
            }
            generate::run(&dir.unwrap_or_else(|| ".".into()), check)
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
