use crate::engine::pictures::remember;
use crate::engine::renpy::python::{Interpreter, find_interpreter};
use crate::engine::renpy::switch::switch_file;
use crate::engine::renpy::{
    ARCHIVES, ENGINE_DIR, GAME_DIR, LIB_DIR, PICTURES, READING, SCRIPTS, STEPS, TEXT, TL_DIR,
    WORKING, archive, chosen, compiled, has_ext, names, parameterized, pictures, shipped,
};
use crate::engine::{Prepare, fonts as face};
use crate::hash::{Rolling, xxh3};
use crate::progress::{Progress, Source};
use crate::scope::slashed;
use crate::{store, walk};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tokio::fs;
use tokio::process::Command;

static TOOLS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/resources/renpy");

static RE_VERSION_STRING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"version\s*=\s*['"](\d+)\.(\d+)\.(\d+)"#)
        .expect("RE_VERSION_STRING is a valid pattern")
});

static RE_VERSION_TUPLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"version_tuple\s*=\s*\((\d+),\s*(\d+),\s*(\d+)")
        .expect("RE_VERSION_TUPLE is a valid pattern")
});

#[tracing::instrument(name = "renpy.prepare", skip_all)]
pub async fn run(at: Prepare<'_>) -> Result<()> {
    at.progress.stage(&[READING], 0);

    let found = inspect(at.game_dir).await;

    let opening = !found.archives.is_empty();
    let recovering = opening || found.compiled_only > 0;

    let plan = planned(opening, recovering);

    at.progress.stage(&plan, 0);
    at.progress.info(Source::Prepare, &found.label);

    if found.packed > 0 {
        at.progress.info(
            Source::Prepare,
            &format!(
                "{} archive(s) hold art and sound only, and stay packed: their pictures are read \
                 where they sit",
                found.packed
            ),
        );
    }

    let python = find_interpreter(at.game_dir).await?;

    if recovering {
        let tools = store::tools_dir()?;
        stage_tools(&tools).await?;

        if opening {
            at.progress.stage(&plan, stage_of(&plan, ARCHIVES));
            extract_archives(&python, &tools, &found.archives, at.progress).await?;
            at.progress.info(
                Source::Prepare,
                &format!("{} archive(s) opened", found.archives.len()),
            );
        }

        at.progress.stage(&plan, stage_of(&plan, SCRIPTS));

        let orphans = compiled::orphans(&at.game_dir.join(GAME_DIR)).await;
        decompile(&python, &tools, at.game_dir, &orphans, at.progress).await?;
        at.progress.info(
            Source::Prepare,
            &match orphans.len() {
                0 => "every script in this game came with its own source, so none was read back"
                    .to_string(),
                many => format!(
                    "{many} script(s) came without their source and were read back, and every \
                     script this game ships was left as it is"
                ),
            },
        );
    }

    at.progress.stage(&plan, stage_of(&plan, TEXT));
    extract_source(&python, at.game_dir, at.source, at.progress).await?;
    added(
        at.game_dir,
        at.source,
        at.progress,
        names::add,
        "character name(s) the game never offered for translation",
    )
    .await?;
    added(
        at.game_dir,
        at.source,
        at.progress,
        parameterized::add,
        "line(s) the game draws outside its dialogue box and never offered for \
         translation",
    )
    .await?;

    let name = chosen(at.tweaks);
    if !name.is_empty() {
        read_shipped(at.game_dir, at.source, name, at.progress).await?;
    }

    at.progress.stage(&plan, stage_of(&plan, PICTURES));
    remember_pictures(&at).await?;

    Ok(())
}

fn stage_of(plan: &[&str], step: &str) -> usize {
    plan.iter().position(|held| *held == step).unwrap_or(0)
}

fn planned(opening: bool, recovering: bool) -> Vec<&'static str> {
    STEPS
        .iter()
        .copied()
        .filter(|one| match *one {
            ARCHIVES => opening,
            SCRIPTS => recovering,
            _ => true,
        })
        .collect()
}

async fn added(
    game_dir: &Path,
    source: &Path,
    progress: &dyn Progress,
    add: fn(&Path, &Path) -> Result<u32>,
    what: &str,
) -> Result<()> {
    let here = game_dir.to_path_buf();
    let into = source.to_path_buf();

    let count = tokio::task::spawn_blocking(move || add(&into, &here)).await??;
    if count > 0 {
        progress.info(Source::Prepare, &format!("{count} {what}"));
    }

    Ok(())
}

async fn read_shipped(
    game_dir: &Path,
    source: &Path,
    name: &str,
    progress: &dyn Progress,
) -> Result<()> {
    let here = game_dir.to_path_buf();
    let into = source.to_path_buf();
    let folder = name.to_string();

    let counted =
        tokio::task::spawn_blocking(move || shipped::apply(&into, &here, &folder)).await??;

    if counted.taken == 0 {
        progress.warn(
            Source::Prepare,
            &format!(
                "game/tl/{name} holds {} line(s) but none of them line up with this build: the \
                 game's original script is being translated instead",
                counted.lines
            ),
        );

        return Ok(());
    }

    progress.info(
        Source::Prepare,
        &format!(
            "{} line(s) read from game/tl/{name}, {} kept from the game's original script",
            counted.taken, counted.kept
        ),
    );

    Ok(())
}

struct Found {
    label: String,
    archives: Vec<PathBuf>,
    packed: usize,
    compiled_only: u32,
}

async fn inspect(game_dir: &Path) -> Found {
    let files = walk::files(&game_dir.join(GAME_DIR)).await;

    let compiled_only = compiled::among(&files).await.len() as u32;

    let label = match version(game_dir).await {
        Some((major, minor, patch)) => format!("Ren'Py {major}.{minor}.{patch}"),
        None => "Ren'Py".to_string(),
    };

    let mut archives = Vec::new();
    let mut packed = 0;

    for at in files.into_iter().filter(|path| has_ext(path, "rpa")) {
        let held = at.clone();
        match tokio::task::spawn_blocking(move || holds_words(&held))
            .await
            .unwrap_or(true)
        {
            true => archives.push(at),
            false => packed += 1,
        }
    }

    Found {
        label,
        archives,
        packed,
        compiled_only,
    }
}

fn numbers(pattern: &Regex, text: &str) -> Vec<(u32, u32, u32)> {
    pattern
        .captures_iter(text)
        .filter_map(|found| {
            Some((
                found[1].parse().ok()?,
                found[2].parse().ok()?,
                found[3].parse().ok()?,
            ))
        })
        .collect()
}

async fn runs_python_two(root: &Path) -> bool {
    fs::metadata(root.join(LIB_DIR).join("python2.7"))
        .await
        .is_ok_and(|at| at.is_dir())
}

async fn version(root: &Path) -> Option<(u32, u32, u32)> {
    let read = |name: &str| {
        let at = root.join(ENGINE_DIR).join(name);
        async move { fs::read_to_string(at).await.ok() }
    };

    if let Some(text) = read("vc_version.py").await
        && let Some(found) = numbers(&RE_VERSION_STRING, &text).pop()
    {
        return Some(found);
    }

    let declared = numbers(&RE_VERSION_TUPLE, &read("__init__.py").await?);

    if runs_python_two(root).await {
        declared.into_iter().next()
    } else {
        declared.into_iter().last()
    }
}

async fn remember_pictures(at: &Prepare<'_>) -> Result<()> {
    let game_dir = at.game_dir.to_path_buf();
    let held = tokio::task::spawn_blocking(move || pictures::shots(&game_dir)).await?;

    remember(at, &pictures::LEDGER, &held.shots, &held.shut).await
}

async fn stage_tools(into: &Path) -> Result<()> {
    if laid_stamp(into).await == *BUNDLED {
        return Ok(());
    }

    let _ = fs::remove_dir_all(into).await;
    fs::create_dir_all(into)
        .await
        .with_context(|| format!("creating {}", into.display()))?;

    let laid = into.to_path_buf();
    tokio::task::spawn_blocking(move || TOOLS.extract(&laid))
        .await?
        .with_context(|| "laying out the bundled Ren'Py tools")?;

    Ok(())
}

fn stamp_over(mut each: Vec<(String, String)>) -> String {
    each.sort();

    let mut hash = Rolling::default();
    for (at, body) in each {
        hash.push(at.as_bytes());
        hash.push(body.as_bytes());
    }

    hash.done()
}

static BUNDLED: LazyLock<String> = LazyLock::new(|| {
    let mut each = Vec::new();
    took_in(&TOOLS, &mut each);

    stamp_over(each)
});

fn took_in(dir: &include_dir::Dir<'_>, each: &mut Vec<(String, String)>) {
    for file in dir.files() {
        each.push((slashed(file.path()), xxh3(file.contents())));
    }

    for held in dir.dirs() {
        took_in(held, each);
    }
}

async fn laid_stamp(into: &Path) -> String {
    let mut each = Vec::new();

    for relative in walk::relative(into).await {
        let Ok(body) = fs::read(into.join(&relative)).await else {
            return String::new();
        };

        each.push((slashed(&relative), xxh3(body)));
    }

    stamp_over(each)
}

const WORDS: [&str; 4] = ["rpy", "rpyc", "rpym", "rpymc"];

fn holds_words(at: &Path) -> bool {
    let Ok(index) = archive::listed(at) else {
        return true;
    };

    index.keys().any(|inside| {
        let named = Path::new(inside);
        WORDS.iter().any(|kind| has_ext(named, kind)) || face::is_face(named)
    })
}

async fn extract_archives(
    python: &Interpreter,
    tools: &Path,
    archives: &[PathBuf],
    progress: &dyn Progress,
) -> Result<()> {
    for archive in archives {
        let Some(into) = archive.parent() else {
            continue;
        };
        run_tool(
            python
                .script(&tools.join("rpatool.py"))
                .arg("-x")
                .arg(archive)
                .arg("-o")
                .arg(into)
                .current_dir(into),
            "rpatool",
            progress,
        )
        .await?;
    }

    Ok(())
}

const IN_ONE_GO: usize = 64;

fn unrpyc_at(tools: &Path, python: &Interpreter) -> PathBuf {
    let held = match python.major >= 3 {
        true => "unrpyc",
        false => "unrpyc-legacy",
    };

    tools.join(held).join("unrpyc.py")
}

async fn decompile(
    python: &Interpreter,
    tools: &Path,
    game: &Path,
    orphans: &[PathBuf],
    progress: &dyn Progress,
) -> Result<()> {
    let inner = game.join(GAME_DIR);

    for held in orphans.chunks(IN_ONE_GO) {
        run_tool(
            python
                .script(&unrpyc_at(tools, python))
                .args(held)
                .current_dir(&inner),
            "unrpyc",
            progress,
        )
        .await?;
    }

    Ok(())
}

async fn extract_source(
    python: &Interpreter,
    game: &Path,
    into: &Path,
    progress: &dyn Progress,
) -> Result<()> {
    compiled::dropped(&switch_file(game)).await?;

    let entry = find_entry(game).await.with_context(|| {
        format!(
            "no Ren'Py entry script found in {}: laying out the translation files runs the game's \
             own engine",
            game.display()
        )
    })?;

    let made = game.join(GAME_DIR).join(TL_DIR).join(WORKING);
    walk::cleared(&made).await?;

    run_tool(
        python
            .script(&entry)
            .arg(game)
            .arg("translate")
            .arg(WORKING),
        "renpy translate",
        progress,
    )
    .await?;

    if !made.is_dir() {
        bail!("Ren'Py laid out no translation files in {}", made.display());
    }

    walk::copy(&made, into, |_| true).await?;
    walk::cleared(&made).await?;

    Ok(())
}

async fn find_entry(game: &Path) -> Option<PathBuf> {
    let mut reader = fs::read_dir(game).await.ok()?;
    let mut found = Vec::new();

    while let Ok(Some(entry)) = reader.next_entry().await {
        let at = entry.path();
        if has_ext(&at, "py") && entry.file_type().await.is_ok_and(|kind| kind.is_file()) {
            found.push(at);
        }
    }

    found.sort();
    found.into_iter().next()
}

async fn run_tool(command: &mut Command, tool: &str, progress: &dyn Progress) -> Result<()> {
    let output = command
        .output()
        .await
        .with_context(|| format!("running {tool}"))?;

    if output.status.success() {
        return Ok(());
    }

    for stream in [&output.stdout, &output.stderr] {
        for line in String::from_utf8_lossy(stream).lines() {
            let line = line.trim_end();
            if !line.is_empty() {
                progress.failed(Source::Prepare, &anyhow::anyhow!("{tool}: {line}"));
            }
        }
    }

    bail!("{tool} failed ({})", output.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_tool_somebody_wrote_over_is_laid_out_again() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let into = sandbox.path().join("tools");

        stage_tools(&into).await.expect("the tools are laid out");
        assert_eq!(
            laid_stamp(&into).await,
            *BUNDLED,
            "a fresh lay has to answer to the bundle it came out of, or every prepare after this \
             one throws the tools away and lays them again"
        );

        let mark = into.join("rpatool.py");
        let shipped = fs::read(&mark).await.expect("a bundled tool");
        fs::write(&mark, b"nonsense somebody pasted in")
            .await
            .expect("a file");

        assert_ne!(
            laid_stamp(&into).await,
            *BUNDLED,
            "the mark is taken from what sits on disk, so a hand that edits a tool has to move it"
        );

        stage_tools(&into)
            .await
            .expect("the tools are laid out again");
        assert_eq!(
            fs::read(&mark).await.expect("a bundled tool"),
            shipped,
            "a tool the game never shipped is what unpacks the player's archives, so an edited \
             one has to be put back rather than run"
        );
    }

    #[tokio::test]
    async fn only_the_compiled_scripts_that_came_without_a_source_are_read_back() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let inner = sandbox.path().join(GAME_DIR);
        std::fs::create_dir_all(&inner).expect("a game folder");

        for at in [
            "options.rpy",
            "options.rpyc",
            "packed.rpyc",
            "newer_ren.py",
            "newer.rpyc",
            "module.rpym",
            "module.rpymc",
            "alone.rpymc",
        ] {
            std::fs::write(inner.join(at), b"pretend a script").expect("a game file");
        }

        assert_eq!(
            compiled::orphans(&inner).await,
            [inner.join("alone.rpymc"), inner.join("packed.rpyc")],
            "Ren'Py loads the script a game ships as soon as its digest moves, so writing over it \
             hands the player decompiler output the game never shipped, and it buys nothing: a \
             script that came with its source can already be read for its words. A compiled \
             script beside a _ren.py source is left alone too, because Ren'Py refuses to run a \
             game holding both spellings of one name"
        );
    }

    #[test]
    fn an_archive_that_holds_only_art_stays_packed_and_one_holding_a_script_is_opened() {
        let sandbox = tempfile::tempdir().expect("a temp folder");

        let art = sandbox.path().join("art.rpa");
        std::fs::write(
            &art,
            archive::sealed(
                &[
                    ("art/shot/day.png", b"pretend a picture", 0),
                    ("audio/theme.ogg", b"pretend a song", 0),
                ],
                0x4242_4242,
                false,
            ),
        )
        .expect("an archive");
        assert!(
            !holds_words(&art),
            "unpacking a gigabyte of art buys nothing: the reader never translates it as text, \
             the game keeps loading it from the archive, and the copy on disk is dead weight"
        );

        let scripts = sandbox.path().join("scripts.rpa");
        std::fs::write(
            &scripts,
            archive::sealed(
                &[
                    ("art/shot/day.png", b"pretend a picture", 0),
                    ("script.rpyc", b"pretend a script", 0),
                ],
                0x4242_4242,
                false,
            ),
        )
        .expect("an archive");
        assert!(
            holds_words(&scripts),
            "the words are the whole point, so an archive carrying any script has to come out"
        );

        let fonts = sandbox.path().join("fonts.rpa");
        std::fs::write(
            &fonts,
            archive::sealed(&[("gui/Lato.ttf", b"pretend a face", 0)], 0, true),
        )
        .expect("an archive");
        assert!(
            holds_words(&fonts),
            "a face has to be on disk to be read and swapped for one that draws the language"
        );

        let broken = sandbox.path().join("broken.rpa");
        std::fs::write(&broken, b"not an archive this reader knows").expect("a file");
        assert!(
            holds_words(&broken),
            "an archive this reader cannot list is opened the old way rather than skipped, or a \
             format we have not met yet would silently lose its scripts"
        );
    }

    #[test]
    fn only_the_steps_that_will_run_are_shown() {
        use crate::engine::renpy::{PICTURES, READING, TEXT};

        assert_eq!(
            planned(false, false),
            [READING, TEXT, PICTURES],
            "a game that ships plain .rpy files opens no archive and decompiles nothing, and the \
             reader watches every step tick over: one that never ran may not be ticked"
        );
        assert_eq!(planned(false, true), [READING, SCRIPTS, TEXT, PICTURES]);
        assert_eq!(
            planned(true, true),
            [READING, ARCHIVES, SCRIPTS, TEXT, PICTURES]
        );
        assert_eq!(
            planned(true, true).len(),
            STEPS.len(),
            "with everything to do the plan is the whole list, so no step is ever invented"
        );
    }

    async fn sandbox(both: bool, python_two: bool) -> tempfile::TempDir {
        let at = tempfile::tempdir().expect("a temp folder");

        let mut body = String::from("if PY2:\n    version_tuple = (7, 4, 11, vc_version)\n");
        if both {
            body.push_str("else:\n    version_tuple = (8, 0, 0, vc_version)\n");
        }

        fs::create_dir_all(at.path().join("renpy")).await.unwrap();
        fs::write(at.path().join("renpy").join("__init__.py"), body)
            .await
            .unwrap();

        if python_two {
            fs::create_dir_all(at.path().join("lib").join("python2.7"))
                .await
                .unwrap();
        }

        at
    }

    #[tokio::test]
    async fn a_game_on_python_two_is_the_seven_line_it_actually_runs() {
        let at = sandbox(true, true).await;

        assert_eq!(
            version(at.path()).await,
            Some((7, 4, 11)),
            "__init__.py declares one version per Python line; shipping python2.7 picks the first"
        );
    }

    #[tokio::test]
    async fn a_game_without_python_two_is_the_later_line() {
        let at = sandbox(true, false).await;

        assert_eq!(version(at.path()).await, Some((8, 0, 0)));
    }
}
