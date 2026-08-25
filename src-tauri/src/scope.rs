use anyhow::{Result, bail};
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub fn slashed(at: &Path) -> String {
    at.to_string_lossy().replace('\\', "/")
}

pub fn key(root: &Path, file: &Path) -> String {
    slashed(file.strip_prefix(root).unwrap_or(file))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope(String);

impl Scope {
    pub fn read(asked: &str) -> Result<Self> {
        let mut cleaned = String::new();

        for part in asked.split(['/', '\\']).filter(|part| !part.is_empty()) {
            let mut walk = Path::new(part).components();

            match (walk.next(), walk.next()) {
                (Some(Component::Normal(_)), None) => {}
                _ => bail!("{asked} is not part of this game's text"),
            }

            if !cleaned.is_empty() {
                cleaned.push('/');
            }
            cleaned.push_str(part);
        }

        Ok(Self(cleaned))
    }

    pub fn everything(&self) -> bool {
        self.0.is_empty()
    }

    pub fn holds(&self, key: &str) -> bool {
        self.everything()
            || key == self.0
            || key
                .strip_prefix(&self.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    pub fn under(&self, source: &Path) -> PathBuf {
        let mut at = source.to_path_buf();
        for part in self.0.split('/').filter(|part| !part.is_empty()) {
            at.push(part);
        }

        at
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn leaf(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

pub fn reach(asked: &[String]) -> Result<Vec<Scope>> {
    asked.iter().map(|one| Scope::read(one)).collect()
}

pub fn named(reach: &[Scope]) -> String {
    match reach {
        [] => "nothing".to_string(),
        [one] => one.to_string(),
        many => format!("{} of the places listed", many.len()),
    }
}

pub fn anywhere(reach: &[Scope]) -> bool {
    reach.iter().any(Scope::everything)
}

pub fn prefixed(reach: &[Scope], what: String) -> String {
    match anywhere(reach) {
        true => what,
        false => format!("{}: {what}", named(reach)),
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(if self.everything() {
            "the whole game"
        } else {
            &self.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(asked: &str) -> Scope {
        Scope::read(asked).expect("a plain key")
    }

    #[test]
    fn everything_is_the_empty_scope() {
        let all = Scope::default();

        assert!(all.everything());
        assert!(all.holds("data/Map001.json"));
        assert_eq!(
            all.under(Path::new("/store/source")),
            Path::new("/store/source")
        );
        assert_eq!(all.to_string(), "the whole game");
    }

    #[test]
    fn a_folder_holds_what_sits_below_it_and_nothing_else() {
        let folder = scope("text_asset/resources.assets");

        assert!(folder.holds("text_asset/resources.assets"));
        assert!(folder.holds("text_asset/resources.assets/scene051.txt"));
        assert!(
            !folder.holds("text_asset/resources.assets2/scene051.txt"),
            "a shared prefix is not the same folder"
        );
        assert!(!folder.holds("text_asset/sharedassets0.assets/mom.txt"));
    }

    #[test]
    fn one_file_holds_only_itself() {
        let file = scope("string_table/en/UI.sheet");

        assert!(file.holds("string_table/en/UI.sheet"));
        assert!(!file.holds("string_table/en/UI.sheet.bak"));
        assert!(!file.holds("string_table/en"));
    }

    #[test]
    fn a_key_lands_under_the_source_folder_it_came_from() {
        let source = Path::new("/store/source");

        assert_eq!(
            scope("text_asset/resources.assets/scene051.txt").under(source),
            source.join("text_asset/resources.assets/scene051.txt")
        );
    }

    #[test]
    fn a_key_that_could_climb_out_is_refused() {
        for asked in ["..", "../../etc/passwd", "text_asset/../../etc", "."] {
            assert!(
                Scope::read(asked).is_err(),
                "{asked} must never reach the filesystem"
            );
        }
    }

    #[test]
    fn separators_are_the_same_on_every_platform() {
        assert_eq!(slashed(Path::new("data\\Map001.json")), "data/Map001.json");
        assert_eq!(
            key(
                Path::new("/store/source"),
                Path::new("/store/source/data/Map001.json")
            ),
            "data/Map001.json"
        );
        assert_eq!(scope("data\\Map001.json").as_str(), "data/Map001.json");
        assert_eq!(scope("/data//Map001.json/").as_str(), "data/Map001.json");
    }

    #[test]
    fn a_reach_names_the_places_an_action_was_asked_for() {
        let places =
            reach(&["a/one.sheet".to_string(), "b/two.sheet".to_string()]).expect("two places");

        assert_eq!(named(&places), "2 of the places listed");
        assert!(!anywhere(&places));

        assert!(
            anywhere(&reach(&[String::new()]).expect("the whole game")),
            "an action on the game itself names one scope that holds every key"
        );
    }

    #[test]
    fn an_empty_reach_asks_for_nothing() {
        assert!(
            !anywhere(&[]),
            "naming no place must never be read as naming every place"
        );
        assert_eq!(named(&[]), "nothing");
    }

    #[test]
    fn a_key_that_looks_absolute_still_lands_inside_the_source_folder() {
        let source = Path::new("/store/source");

        assert_eq!(
            scope("/etc/passwd").under(source),
            source.join("etc").join("passwd"),
            "a key is read out of a ledger on disk, so a leading separator has to be dropped \
             rather than left to be read as a root of its own"
        );
    }

    #[test]
    fn a_file_is_named_after_the_last_step_of_its_key() {
        assert_eq!(
            leaf("text_asset/resources.assets/scene051.txt"),
            "scene051.txt"
        );
        assert_eq!(leaf("script.rpy"), "script.rpy");
        assert_eq!(leaf(""), "");
    }
}
