use crate::engine::renpy::LIB_DIR;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;

pub struct Interpreter {
    binary: PathBuf,
    stdlib: Option<Stdlib>,
    pub major: u32,
}

#[derive(Clone)]
struct Stdlib {
    at: PathBuf,
    optimized: bool,
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command(binary: &Path, stdlib: Option<&Stdlib>) -> Command {
    let mut command = Command::new(binary);

    #[cfg(target_os = "linux")]
    if let Some(folder) = binary.parent() {
        command.env("LD_LIBRARY_PATH", folder);
    }

    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    if let Some(stdlib) = stdlib {
        command.env("PYTHONPATH", &stdlib.at);
        command.arg("-S");

        if stdlib.optimized {
            command.arg("-O");
        }
    }

    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command.env("PYTHONIOENCODING", "utf-8");

    command
}

impl Interpreter {
    pub fn script(&self, tool: &Path) -> Command {
        let mut command = command(&self.binary, self.stdlib.as_ref());
        command.arg(tool);

        command
    }
}

fn named(at: &Path) -> String {
    at.file_name().unwrap_or_default().to_string_lossy().into()
}

#[cfg(target_os = "linux")]
const PYTHON: &str = "python";

#[cfg(target_os = "windows")]
const PYTHON: &str = "python.exe";

fn ranked(build: &str) -> (u8, u8) {
    let foreign = u8::from(!build.contains(std::env::consts::ARCH));

    let older_line = if build.starts_with("py3-") {
        0
    } else if build.starts_with("py2-") {
        1
    } else {
        2
    };

    (foreign, older_line)
}

async fn candidates(lib: &Path) -> Vec<PathBuf> {
    let Ok(mut reader) = fs::read_dir(lib).await else {
        return Vec::new();
    };

    let mut found = Vec::new();

    while let Ok(Some(entry)) = reader.next_entry().await {
        let build = entry.path();
        let binary = build.join(PYTHON);

        if fs::metadata(&binary).await.is_ok_and(|at| at.is_file()) {
            found.push((ranked(&named(&build)), binary));
        }
    }

    found.sort();
    found.into_iter().map(|(_, binary)| binary).collect()
}

async fn stdlibs(lib: &Path) -> Vec<Stdlib> {
    let Ok(mut reader) = fs::read_dir(lib).await else {
        return Vec::new();
    };

    let mut found = Vec::new();

    while let Ok(Some(entry)) = reader.next_entry().await {
        let at = entry.path();

        if named(&at).starts_with("python")
            && entry.file_type().await.is_ok_and(|kind| kind.is_dir())
        {
            let optimized = needs_optimized(&at).await;
            found.push(Stdlib { at, optimized });
        }
    }

    found.sort_by(|one, other| one.at.cmp(&other.at));
    found
}

async fn needs_optimized(at: &Path) -> bool {
    let Ok(mut reader) = fs::read_dir(at).await else {
        return false;
    };

    while let Ok(Some(entry)) = reader.next_entry().await {
        if entry.path().extension().is_some_and(|kind| kind == "pyo") {
            return true;
        }
    }

    false
}

#[cfg(target_os = "linux")]
async fn make_runnable(binary: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::metadata(binary).await else {
        return;
    };

    let mut permissions = metadata.permissions();
    if permissions.mode() & 0o111 == 0 {
        permissions.set_mode(permissions.mode() | 0o755);
        let _ = fs::set_permissions(binary, permissions).await;
    }
}

const PROBE: &str = "import argparse, sys; print(sys.version_info[0])";

async fn started(binary: &Path, stdlib: Option<&Stdlib>) -> Option<Interpreter> {
    let output = command(binary, stdlib)
        .arg("-c")
        .arg(PROBE)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let major = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;

    Some(Interpreter {
        binary: binary.to_path_buf(),
        stdlib: stdlib.cloned(),
        major,
    })
}

async fn started_in(lib: &Path) -> Option<Interpreter> {
    let shipped = stdlibs(lib).await;

    for binary in candidates(lib).await {
        #[cfg(target_os = "linux")]
        make_runnable(&binary).await;

        if let Some(alone) = started(&binary, None).await {
            return Some(alone);
        }

        for stdlib in &shipped {
            if let Some(pointed) = started(&binary, Some(stdlib)).await {
                return Some(pointed);
            }
        }
    }

    None
}

pub async fn find_interpreter(root: &Path) -> Result<Interpreter> {
    let lib = root.join(LIB_DIR);

    match started_in(&lib).await {
        Some(found) => Ok(found),
        None => bail!("no Python under {} would start", lib.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spelled(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect()
    }

    fn handed(command: &Command, name: &str) -> Option<String> {
        command.as_std().get_envs().find_map(|(key, value)| {
            (key == name).then(|| value.unwrap_or_default().to_string_lossy().to_string())
        })
    }

    fn interpreter(stdlib: Option<Stdlib>) -> Interpreter {
        Interpreter {
            binary: PathBuf::from("/game/lib/linux-x86_64/python"),
            stdlib,
            major: 2,
        }
    }

    #[test]
    fn a_tool_is_handed_over_as_the_script_being_run() {
        let running = interpreter(None).script(Path::new("/tools/unrpyc/unrpyc.py"));

        assert_eq!(
            spelled(&running),
            ["/tools/unrpyc/unrpyc.py"],
            "unrpyc hands its files to a pool of processes, and a pool pickles the worker by the \
             name of the module it came out of. Reading the tool and exec'ing it under a borrowed \
             __main__ leaves that name pointing at nothing, so every read dies on the first \
             script. Run as a script the tool is __main__, and Python puts its own folder on the \
             path for us"
        );
        assert_eq!(
            handed(&running, "PYTHONIOENCODING").as_deref(),
            Some("utf-8"),
            "a Python 2 writing into a pipe takes its encoding from the machine, and on Windows it \
             is handed none at all: the first Japanese file name unrpyc logs ends the run with a \
             UnicodeEncodeError"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            handed(&running, "LD_LIBRARY_PATH").as_deref(),
            Some("/game/lib/linux-x86_64"),
            "the libraries a Ren'Py build was linked against sit beside its interpreter"
        );
        assert_eq!(handed(&running, "PYTHONPATH"), None);
    }

    #[test]
    fn a_python_handed_the_library_the_game_ships_is_started_without_site() {
        let stdlib = Stdlib {
            at: PathBuf::from("/game/lib/pythonlib2.7"),
            optimized: true,
        };

        let running = interpreter(Some(stdlib)).script(Path::new("/tools/unrpyc.py"));

        assert_eq!(
            spelled(&running),
            ["-S", "-O", "/tools/unrpyc.py"],
            "these builds ship the standard library compiled and nothing else: there is no site \
             module to import, and Python 2 reads a .pyo only when it is optimized"
        );
        assert_eq!(
            handed(&running, "PYTHONPATH").as_deref(),
            Some("/game/lib/pythonlib2.7")
        );
    }

    #[test]
    fn a_library_of_plain_bytecode_is_not_optimized_away() {
        let stdlib = Stdlib {
            at: PathBuf::from("/game/lib/python3.12"),
            optimized: false,
        };

        assert_eq!(
            spelled(&interpreter(Some(stdlib)).script(Path::new("/tools/unrpyc.py"))),
            ["-S", "/tools/unrpyc.py"],
            "-O throws away the assertions unrpyc leans on to notice a script it read wrong, so \
             it is spent only where a .pyo library cannot be imported without it"
        );
    }
}
