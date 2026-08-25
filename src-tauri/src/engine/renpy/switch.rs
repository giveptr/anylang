use crate::engine::fonts::Fonts;
use crate::engine::renpy::{GAME_DIR, fonts, slug};
use crate::engine::{Tweaks, quoted};
use std::path::{Path, PathBuf};

pub const SWITCH_FILE: &str = concat!(env!("CARGO_PKG_NAME"), ".rpy");

const FONT_HOOK: &str = concat!("_", env!("CARGO_PKG_NAME"), "_fonts");
const DEVELOPER: &str = "init 1699 python:\n    config.developer = True\n";

pub fn switch_file(game_dir: &Path) -> PathBuf {
    game_dir.join(GAME_DIR).join(SWITCH_FILE)
}

pub fn switch(
    language: &str,
    tweaks: &Tweaks,
    fonts_chosen: &Fonts,
    placed: &[(String, String)],
) -> Option<String> {
    let Tweaks::RenPy(_) = tweaks else {
        return None;
    };

    let mut body = format!(
        "init 9999 python:\n    renpy.game.preferences.language = \"{}\"\n",
        slug(language)
    );

    let carried = |path: &str| {
        let want = path.trim();

        placed
            .iter()
            .find(|(from, _)| from == want)
            .map(|(_, name)| quoted(&fonts::landing_of(language, name)))
    };

    let mut replaces = false;
    let mut mapped: Vec<String> = Vec::new();

    for one in &fonts_chosen.swaps {
        let Some(from) = Path::new(one.from.trim()).file_name() else {
            continue;
        };

        let Some(landing) = carried(&one.to) else {
            continue;
        };

        replaces = true;
        mapped.push(format!("{}: {landing}", quoted(&from.to_string_lossy())));
    }

    if replaces {
        body.push_str(&format!(
            "    {FONT_HOOK}_map = {{{}}}\n\
             \x20   class {FONT_HOOK}(object):\n\
             \x20       def get(self, key, default=None):\n\
             \x20           name = str(key[0]).replace(\"\\\\\", \"/\").rsplit(\"/\", 1)[-1]\n\
             \x20           found = {FONT_HOOK}_map.get(name)\n\
             \x20           if not found:\n\
             \x20               return default\n\
             \x20           return (found, key[1], key[2])\n\
             \x20   config.font_replacement_map = {FONT_HOOK}()\n",
            mapped.join(", ")
        ));
    }

    body.push('\n');
    body.push_str(DEVELOPER);

    Some(body)
}

#[cfg(test)]
pub fn switched(language: &str, tweaks: &Tweaks, fonts_chosen: &Fonts) -> Option<String> {
    switch(
        language,
        tweaks,
        fonts_chosen,
        &fonts::landings(fonts_chosen),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;
    use crate::engine::renpy::Options;

    fn tweaks() -> Tweaks {
        Tweaks::RenPy(Options::default())
    }

    fn swapping(swaps: &[(&str, &str)]) -> Fonts {
        Fonts {
            swaps: swaps
                .iter()
                .map(|(from, to)| Swap {
                    from: (*from).to_string(),
                    to: (*to).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_language_is_chosen_at_the_last_init_ren_py_runs() {
        let body = switched("Japanese", &tweaks(), &Fonts::default()).unwrap();

        assert!(body.starts_with("init 9999 python:\n"));
        assert!(body.contains("preferences.language = \"japanese\""));
    }

    #[test]
    fn a_language_that_would_break_python_reaches_the_file_slugged() {
        let body = switched("ja\"pa", &tweaks(), &Fonts::default()).unwrap();

        assert!(
            body.contains("preferences.language = \"ja_pa\""),
            "the name lands inside a Python string, so a quote in it closes the line early and the \
             whole file stops compiling:\n{body}"
        );
    }

    #[test]
    fn developer_mode_is_switched_on_before_ren_py_reads_the_flag() {
        let body = switched("Japanese", &tweaks(), &Fonts::default()).unwrap();

        assert!(
            body.contains("init 1699 python:\n    config.developer = True\n"),
            "a translated build is going to hit a missing image or a bad tag, and without the \
             console the reader only sees the game close. Ren'Py loads the developer menu and \
             builds the console at init 1700, reading the flag \
             once, so a flag raised any later leaves the reader with \"Screen _developer is not \
             known\" and a console that never opens:\n{body}"
        );
    }

    #[test]
    fn a_font_is_mapped_over_the_one_it_replaces() {
        let body = switched(
            "Japanese",
            &tweaks(),
            &swapping(&[("gui/old.ttf", "/elsewhere/mine.ttf")]),
        )
        .unwrap();

        assert!(body.contains(&format!(
            "{FONT_HOOK}_map = {{\"old.ttf\": \"tl/japanese/fonts/anylang-mine.ttf\"}}"
        )));
        assert!(body.contains(&format!("config.font_replacement_map = {FONT_HOOK}()")));
        assert!(body.contains("return (found, key[1], key[2])"));
    }

    #[test]
    fn no_font_means_no_replacement_block() {
        let body = switched("Japanese", &tweaks(), &swapping(&[])).unwrap();
        assert!(!body.contains("font_replacement_map"));
    }

    #[test]
    fn a_swap_that_lands_nowhere_leaves_the_games_own_map_alone() {
        let body = switched("Japanese", &tweaks(), &swapping(&[("gui/old.ttf", "/")])).unwrap();

        assert!(
            !body.contains("font_replacement_map"),
            "a map replacing nothing still replaces the game's own, so it may not be written at \
             all: {body}"
        );
    }

    #[test]
    fn each_face_can_be_sent_to_a_different_one() {
        let body = switched(
            "French",
            &tweaks(),
            &swapping(&[
                ("game/gui/Lato-Regular.ttf", "/x/NotoSans.ttf"),
                ("zenda.ttf", "/y/Charm-Bold.otf"),
            ]),
        )
        .expect("a switch file");

        assert!(
            body.contains(
                r#"_anylang_fonts_map = {"Lato-Regular.ttf": "tl/french/fonts/anylang-NotoSans.ttf", "zenda.ttf": "tl/french/fonts/anylang-Charm-Bold.otf"}"#
            ),
            "each face the game asks for has to reach the one chosen for it:\n{body}"
        );
    }

    #[test]
    fn a_face_left_empty_is_not_written_down() {
        let body = switched(
            "French",
            &tweaks(),
            &swapping(&[("Prince Valiant.ttf", ""), ("other.ttf", "/x/NotoSans.ttf")]),
        )
        .expect("a switch file");

        assert!(
            body.contains(
                r#"_anylang_fonts_map = {"other.ttf": "tl/french/fonts/anylang-NotoSans.ttf"}"#
            ),
            "a row holding nothing is a row the reader never filled, so only the filled one \
             reaches the map:\n{body}"
        );
    }
}
