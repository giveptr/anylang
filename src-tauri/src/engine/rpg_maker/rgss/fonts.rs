use crate::engine::fonts::Fonts;
use crate::engine::pictures::key_of;
use crate::engine::rpg_maker::rgss::{archive, scripts, source};
use crate::engine::{Extra, Font, Install, fonts as face, quoted};
use anyhow::Result;
use std::path::{Path, PathBuf};

const DIR: &str = "Fonts";
const BOOT: &str = "rgss_main";
const MARK: &str = "patch_font";
const TABLE: &str = "PATCH_FONTS";
const EVERY: &str = "PATCH_FONT_ALL";

fn packed(game_dir: &Path, store: &Path) -> (Vec<Font>, Vec<PathBuf>) {
    let mut found = Vec::new();
    let mut lifted = Vec::new();

    let Some(at) = source::packed_at(game_dir) else {
        return (found, lifted);
    };
    let from = face::untouched(store, game_dir, &at);
    let Ok(entries) = archive::opened(&from) else {
        return (found, lifted);
    };
    let holder = at
        .file_name()
        .map(|one| one.to_string_lossy().to_string())
        .unwrap_or_default();

    for one in entries {
        let inside = Path::new(&one.name);
        let named = face::by_name(inside).unwrap_or_default();

        if !one.name.starts_with(&format!("{DIR}/")) || !face::is_face(inside) || face::ours(&named)
        {
            continue;
        }

        let at = key_of(&holder, Some(&one.name));
        let Ok(body) = archive::read(&from, &one) else {
            continue;
        };
        let Some(copy) = face::lift(store, &at, &body) else {
            continue;
        };

        found.push(Font {
            name: named,
            at,
            shown: copy.to_string_lossy().to_string(),
            builtin: false,
        });
        lifted.push(copy);
    }

    (found, lifted)
}

pub fn faces(game_dir: &Path, store: &Path) -> Vec<Font> {
    let mut found = face::faces(game_dir, &game_dir.join(DIR));
    found.retain(|one| !face::ours(&one.name));

    let (packed, lifted) = packed(game_dir, store);
    face::swept(store, &lifted);

    found.retain(|loose| !packed.iter().any(|one| one.name == loose.name));
    found.extend(packed);
    found.sort_by(|a, b| a.name.cmp(&b.name));

    found
}

pub async fn tidied(at: &Install<'_>) -> u32 {
    face::tidied(&at.game_dir.join(DIR), at).await
}

pub fn carried(fonts: &Fonts) -> Vec<Extra> {
    face::carried(&face::landed(fonts), Path::new(DIR))
}

fn table(each: &[(String, String)]) -> String {
    let each: Vec<String> = each
        .iter()
        .map(|(from, to)| format!("{} => {}", quoted(from), quoted(to)))
        .collect();

    format!("{{{}}}", each.join(", "))
}

fn hook(sending: &Sending) -> String {
    let each = table(&sending.each);
    let all = sending
        .all
        .as_deref()
        .map(quoted)
        .unwrap_or_else(|| "nil".to_string());

    [
        "class Font".to_string(),
        format!("  {TABLE} = {each}"),
        format!("  {EVERY} = {all}"),
        String::new(),
        format!("  unless method_defined?(:{MARK}_name=)"),
        format!("    alias_method :{MARK}_name=, :name="),
        format!("    alias_method :{MARK}_init, :initialize"),
        "  end".to_string(),
        String::new(),
        format!("  def self.{MARK}_pick(value)"),
        "    named = value.nil? ? default_name : value".to_string(),
        "    named = [named] unless named.is_a?(Array)".to_string(),
        "    named.map do |one|".to_string(),
        format!("      if {TABLE}.has_key?(one.to_s)"),
        format!("        {TABLE}[one.to_s]"),
        format!("      elsif {TABLE}.has_value?(one.to_s) || one.to_s == {EVERY}"),
        "        one".to_string(),
        "      else".to_string(),
        format!("        {EVERY} || one"),
        "      end".to_string(),
        "    end".to_string(),
        "  end".to_string(),
        String::new(),
        "  def name=(value)".to_string(),
        format!("    self.{MARK}_name = Font.{MARK}_pick(value)"),
        "  end".to_string(),
        String::new(),
        "  def initialize(*args)".to_string(),
        format!("    {MARK}_init(*args)"),
        "    self.name = args[0] unless args.empty?".to_string(),
        "  end".to_string(),
        "end".to_string(),
        format!("Font.default_name = Font.{MARK}_pick(nil)"),
        String::new(),
    ]
    .join("\n")
}

fn boots(each: &[(usize, String, String)]) -> Option<usize> {
    let found = each
        .iter()
        .position(|(_, _, source)| source.contains(BOOT))
        .or_else(|| {
            each.iter()
                .rposition(|(_, _, source)| !source.trim().is_empty())
        })?;

    each.get(found).map(|(which, _, _)| *which)
}

fn seam(source: &str) -> usize {
    let Some(boot) = source.find(BOOT) else {
        return source.len();
    };

    source[..boot].rfind('\n').map_or(0, |line| line + 1)
}

pub fn told(bytes: &[u8], sending: &Sending) -> Result<(Vec<u8>, u32), String> {
    let each = scripts::listed(bytes)?;
    let told = hook(sending);

    let Some(boot) = boots(&each) else {
        return Err("this game holds no script to tell a font to".to_string());
    };

    scripts::rewritten(bytes, |which, _, source| {
        if which != boot {
            return None;
        }

        let (front, back) = source.split_at(seam(source));
        let apart = match front.is_empty() || front.ends_with('\n') {
            true => "",
            false => "\n",
        };

        Some(format!("{front}{apart}{told}{back}"))
    })
}

#[derive(Default)]
pub struct Sending {
    pub each: Vec<(String, String)>,
    pub all: Option<String>,
}

impl Sending {
    pub fn wanted(&self) -> bool {
        self.all.is_some() || !self.each.is_empty()
    }
}

pub async fn sending(at: &Install<'_>) -> Result<Sending> {
    if at.reverting {
        return Ok(Sending::default());
    }

    let held = at.fonts.picked().await?;
    if held.is_empty() {
        return Ok(Sending::default());
    }

    let named = |from: &str| {
        held.get(from)
            .and_then(|body| face::family(body))
            .ok_or_else(|| anyhow::anyhow!("{from} does not say what family it belongs to"))
    };

    let listed = faces(at.game_dir, at.store);
    let all = match at.fonts.one_for_every(&listed) {
        Some(from) => Some(named(from)?),
        None => None,
    };

    let mut asked: Vec<(String, String)> = Vec::new();
    for one in listed {
        let Some(from) = at.fonts.sent_to(&one.name) else {
            continue;
        };
        let to = named(from)?;

        if all.as_deref() == Some(to.as_str()) {
            continue;
        }

        let Some(shipped) = family_of(at, &one).await else {
            continue;
        };

        asked.push((shipped, to));
    }

    Ok(Sending {
        each: pinned(asked, all.as_deref()),
        all,
    })
}

async fn family_of(at: &Install<'_>, one: &Font) -> Option<String> {
    let body = match tokio::fs::read(&one.shown).await {
        Ok(body) => body,
        Err(why) => {
            at.progress.warn(at.doing, &format!("{}: {why}", one.name));

            return None;
        }
    };

    let found = face::family(&body);
    if found.is_none() {
        at.progress.warn(
            at.doing,
            &format!("{} does not say what family it belongs to", one.name),
        );
    }

    found
}

fn pinned(asked: Vec<(String, String)>, all: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    for (shipped, to) in asked {
        if all == Some(to.as_str()) || (all.is_none() && shipped == to) {
            continue;
        }

        if !out.iter().any(|(from, _)| *from == shipped) {
            out.push((shipped, to));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;
    use crate::engine::rpg_maker::rgss::{fixture, packed};
    use crate::progress::Quiet;
    use std::fs;

    fn read(bytes: &[u8]) -> Vec<String> {
        scripts::sources(bytes)
            .expect("the scripts")
            .into_iter()
            .map(|(_, source)| source)
            .collect()
    }

    #[test]
    fn a_face_packed_into_the_archive_is_offered_and_beats_the_loose_copy_beside_it() {
        let at = tempfile::tempdir().expect("a temp folder");
        let store = tempfile::tempdir().expect("a store");
        let game = at.path();
        fs::create_dir_all(game.join(DIR)).expect("a Fonts folder");

        let shipped = face::fake::called("VL Gothic");
        let inert = face::fake::called("Inert");
        fs::write(game.join(DIR).join("VLGothic.ttf"), &inert).expect("a loose font");
        fs::write(
            game.join("Game.rgss3a"),
            archive::packed(&[
                ("Fonts/VLGothic.ttf", shipped.as_slice()),
                ("Fonts/Sazanami.ttf", shipped.as_slice()),
                ("Data/Map001.rvdata2", b"not a font".as_slice()),
            ]),
        )
        .expect("an archive");

        let found = faces(game, store.path());

        assert_eq!(
            found
                .iter()
                .map(|one| (one.name.as_str(), one.at.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Sazanami.ttf", "Game.rgss3a|Fonts/Sazanami.ttf"),
                ("VLGothic.ttf", "Game.rgss3a|Fonts/VLGothic.ttf"),
            ],
            "the game reads the archive before the folder, so the packed copy is the one a \
             reader is replacing and the loose duplicate beside it is inert"
        );
        assert_eq!(
            fs::read(&found[1].shown).expect("the copy lifted out of the archive"),
            shipped,
            "reading the family out of the loose copy would map the wrong name into the hook"
        );
    }

    #[test]
    fn a_game_that_boots_without_rgss_main_still_gets_ruby_that_parses() {
        let raw = fixture::scripts(&[("Main", "SceneManager.run")]);

        let (fresh, written) = told(
            &raw,
            &Sending {
                each: vec![("\u{30d5}".to_string(), "Sans".to_string())],
                all: None,
            },
        )
        .expect("a game told which font to reach for");

        assert_eq!(written, 1);
        assert!(
            !read(&fresh)[0].contains("runclass Font"),
            "the hook goes in at the end of a script that never calls rgss_main, and a script \
             typed without a trailing newline glues the two together into ruby that will not \
             parse: the archive written back is the player's own, and the game stops starting"
        );
    }

    #[test]
    fn the_hook_lands_in_the_boot_script_even_when_a_script_ahead_of_it_cannot_be_read() {
        let unreadable = fixture::list(&[
            fixture::number(0),
            fixture::said("broken"),
            fixture::number(0),
        ]);
        let booting = fixture::list(&[
            fixture::number(1),
            fixture::said("Main"),
            fixture::tagged(
                b'"',
                &packed::shut(b"rgss_main { SceneManager.run }").expect("it packs"),
            ),
        ]);

        let raw = fixture::stream(&fixture::list(&[unreadable, booting]));

        let (fresh, written) = told(
            &raw,
            &Sending {
                each: vec![("\u{30d5}".to_string(), "Sans".to_string())],
                all: None,
            },
        )
        .expect("a game told which font to reach for");

        assert_eq!(
            written, 1,
            "the boot script is found by counting past the ones that would not open, so the hook \
             was aimed at a row of the list that is not there and the game was handed back \
             untouched with nobody told"
        );
        assert!(
            read(&fresh)
                .iter()
                .any(|source| source.contains("class Font") && source.contains("rgss_main")),
            "and it belongs in the script that boots the game, not appended to whichever one \
             happens to sit at that number"
        );
    }

    #[tokio::test]
    async fn a_font_carried_in_under_a_dropped_pick_is_let_go_of_rather_than_left_behind() {
        let at = fixture::sandbox();
        let root = at.path();
        let store = fixture::sandbox();
        let picked = fixture::sandbox();
        fs::create_dir_all(root.join(DIR)).unwrap();

        let font = picked.path().join("Sarabun-Medium.ttf");
        fs::write(&font, face::fake::called("Sarabun Medium")).unwrap();

        let landed = format!("{}-Sarabun-Medium.ttf", face::CARRIED);
        let stale = format!("{}-noto.ttf", face::CARRIED);
        for name in ["patch.ttf", "NIAGSOL.TTF", stale.as_str(), landed.as_str()] {
            fs::write(root.join(DIR).join(name), [0u8; 4]).unwrap();
        }

        let fonts = Fonts {
            swaps: vec![Swap {
                from: "NIAGSOL.TTF".to_string(),
                to: font.to_string_lossy().to_string(),
            }],
        };
        let quiet = Quiet;
        let told = |reverting| {
            Install::over(root, root, store.path())
                .sending(&fonts)
                .putting_back(reverting)
                .heard_by(&quiet)
        };

        let left = || {
            let mut found: Vec<String> = fs::read_dir(root.join(DIR))
                .expect("the folder")
                .filter_map(Result::ok)
                .map(|one| one.file_name().to_string_lossy().to_string())
                .collect();
            found.sort();

            found
        };

        assert_eq!(tidied(&told(false)).await, 1);
        assert_eq!(
            left(),
            vec![
                "NIAGSOL.TTF".to_string(),
                landed.clone(),
                "patch.ttf".to_string(),
            ],
            "an export keeps the font this pick lands as, and the game's own faces: a font we \
             wrote under a name no pick makes any more would keep answering to its family"
        );

        assert_eq!(
            tidied(&told(true)).await,
            1,
            "asking for the game back leaves none of ours behind"
        );
        assert_eq!(
            left(),
            vec!["NIAGSOL.TTF".to_string(), "patch.ttf".to_string()],
            "and a face the game shipped is never one of ours to take"
        );
    }

    fn asking(each: &[(&str, &str)]) -> Vec<(String, String)> {
        each.iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect()
    }

    #[test]
    fn a_face_pinned_to_the_font_it_already_carries_is_still_written_down() {
        let asked = asking(&[("Niagara Solid", "Niagara Solid")]);

        assert_eq!(
            pinned(asked.clone(), Some("Sarabun")),
            asking(&[("Niagara Solid", "Niagara Solid")]),
            "leaving a pick out because it names the face it came from would drop that face to \
             the pick made for everything instead, and the player reads a font they never asked \
             for"
        );

        assert!(
            pinned(asked, None).is_empty(),
            "with nothing made for everything there is no pick to fall to, so a face pinned to \
             itself changes nothing and only grows the hook the game has to run"
        );
    }

    #[test]
    fn a_face_already_carrying_the_pick_made_for_everything_is_left_to_it() {
        assert!(
            pinned(asking(&[("VL Gothic", "Sarabun")]), Some("Sarabun")).is_empty(),
            "the pick made for everything already answers this face, so naming it again writes a \
             line that changes nothing"
        );
    }

    #[test]
    fn one_family_shipped_under_two_names_is_told_once() {
        assert_eq!(
            pinned(
                asking(&[("VL Gothic", "Open Sans"), ("VL Gothic", "Sarabun")]),
                None,
            ),
            asking(&[("VL Gothic", "Open Sans")]),
            "a game can ship one family in several files, and a Ruby hash keeps the last key \
             written, so a second line would quietly undo the first"
        );
    }

    fn told_of(each: &[(&str, &str)], all: Option<&str>) -> Sending {
        Sending {
            each: each
                .iter()
                .map(|(from, to)| (from.to_string(), to.to_string()))
                .collect(),
            all: all.map(str::to_string),
        }
    }

    #[test]
    fn the_font_is_told_to_the_script_that_boots_the_game_not_the_last_one_written() {
        let bytes = fixture::scripts(&[
            ("Cache", "module Cache\r\nend\r\n"),
            (
                "Main",
                "Font.default_name = [\"VL Gothic\"]\r\nrgss_main { SceneManager.run }\r\n",
            ),
            ("Notes", "# nothing here runs, Main never returns\r\n"),
        ]);

        let (fresh, written) =
            told(&bytes, &told_of(&[("VL Gothic", "Open Sans")], None)).expect("a font is told");
        let each = read(&fresh);

        assert_eq!(written, 1);
        assert!(
            each[1].starts_with("Font.default_name = [\"VL Gothic\"]\r\nclass Font\n"),
            "the author names a font on the way in, so the hook has to come after it: {:?}",
            each[1]
        );
        assert!(
            each[1].ends_with("rgss_main { SceneManager.run }\r\n"),
            "and before the call that boots the game, which never returns: {:?}",
            each[1]
        );
        assert_eq!(each[0], "module Cache\r\nend\r\n");
        assert_eq!(each[2], "# nothing here runs, Main never returns\r\n");
    }

    #[test]
    fn a_name_no_font_file_answers_to_falls_to_the_pick_made_for_everything() {
        let told = hook(&told_of(&[("Niagara Solid", "Open Sans")], Some("Sarabun")));

        assert!(
            told.contains("PATCH_FONTS = {\"Niagara Solid\" => \"Open Sans\"}"),
            "a face the game ships is answered by name: {told}"
        );
        assert!(
            told.contains("PATCH_FONT_ALL = \"Sarabun\""),
            "and a name outside that list falls to the pick made for everything: {told}"
        );
        assert!(
            told.contains("PATCH_FONT_ALL || one"),
            "the hook keeps the name the game asked for whenever no pick answers it: {told}"
        );
    }

    #[test]
    fn a_name_answered_by_its_own_pick_never_reaches_the_pick_made_for_everything() {
        let told = hook(&told_of(&[("VL Gothic", "Open Sans")], Some("Sarabun")));

        let by_name = told.find("has_key?(one.to_s)").expect("a lookup by name");
        let already = told.find("has_value?(one.to_s)").expect("a name of ours");
        let fell = told.find("PATCH_FONT_ALL || one").expect("a fall-back");

        assert!(
            by_name < already && already < fell,
            "the hook walks these in order, so a face this reader picked for has to be answered \
             before the name is called unheard-of and handed the pick made for everything: {told}"
        );
    }

    #[test]
    fn a_face_handed_to_font_new_goes_through_the_same_pick_as_one_named_later() {
        let told = hook(&told_of(&[("VL Gothic", "Open Sans")], Some("Sarabun")));

        assert!(
            told.contains("self.name = args[0] unless args.empty?"),
            "a game that builds its font in one go never assigns a name afterwards, so the pick \
             has to happen on the way through Font.new too: {told}"
        );
    }
}
