use crate::scope::Scope;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Picks {
    #[serde(default)]
    pub on: Vec<String>,
    #[serde(default)]
    pub off: Vec<String>,
}

pub struct Ruling {
    on: Vec<Scope>,
    off: Vec<Scope>,
    by_default: bool,
}

impl Picks {
    pub fn ruling(&self, by_default: bool) -> Ruling {
        Ruling {
            on: scopes(&self.on),
            off: scopes(&self.off),
            by_default,
        }
    }

    pub fn set(&mut self, asked: &Scope, on: bool, by_default: bool) {
        for list in [&mut self.on, &mut self.off] {
            list.retain(|pick| !Scope::read(pick).is_ok_and(|one| asked.holds(one.as_str())));
        }

        if self.ruling(by_default).wants(asked.as_str()) == on {
            return;
        }

        let list = if on { &mut self.on } else { &mut self.off };
        list.push(asked.as_str().to_string());
        list.sort();
    }
}

impl Ruling {
    pub fn wants(&self, key: &str) -> bool {
        match (deepest(&self.on, key), deepest(&self.off, key)) {
            (Some(on), Some(off)) => on >= off,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => self.by_default,
        }
    }
}

fn scopes(picks: &[String]) -> Vec<Scope> {
    picks
        .iter()
        .filter_map(|one| Scope::read(one).ok())
        .collect()
}

fn deepest(picks: &[Scope], key: &str) -> Option<usize> {
    picks
        .iter()
        .filter(|one| one.holds(key))
        .map(|one| one.as_str().len())
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(asked: &str) -> Scope {
        Scope::read(asked).expect("a plain key")
    }

    #[test]
    fn nothing_chosen_leaves_every_file_at_the_engines_default() {
        let picks = Picks::default();

        assert!(
            picks
                .ruling(true)
                .wants("mono_behaviour/one.assets/UILabel/7.sheet")
        );
        assert!(
            !picks
                .ruling(false)
                .wants("mono_behaviour/one.assets/UILabel/7.sheet")
        );
    }

    #[test]
    fn the_deepest_choice_is_the_one_that_counts() {
        let mut picks = Picks::default();
        picks.set(&scope("mono_behaviour/one.assets"), true, false);
        picks.set(
            &scope("mono_behaviour/one.assets/SceneImageLoader"),
            false,
            false,
        );

        let ruling = picks.ruling(false);

        assert!(ruling.wants("mono_behaviour/one.assets/UILabel/7.sheet"));
        assert!(
            !ruling.wants("mono_behaviour/one.assets/SceneImageLoader/9.sheet"),
            "turning a class off inside a folder that is on has to win"
        );
        assert!(
            !ruling.wants("mono_behaviour/two.assets/UILabel/7.sheet"),
            "a folder nobody chose stays at the default"
        );
    }

    #[test]
    fn choosing_a_folder_clears_the_choices_made_inside_it() {
        let mut picks = Picks::default();
        picks.set(&scope("mono_behaviour/one.assets/A"), true, false);
        picks.set(&scope("mono_behaviour/one.assets/B"), true, false);
        picks.set(&scope("mono_behaviour/one.assets"), true, false);

        assert_eq!(
            picks.on,
            ["mono_behaviour/one.assets"],
            "the two entries below it can only say the same thing twice"
        );
        assert!(picks.off.is_empty());
    }

    #[test]
    fn a_choice_that_only_repeats_the_default_is_not_written_down() {
        let mut picks = Picks::default();

        picks.set(&scope("mono_behaviour/one.assets"), false, false);
        assert!(
            picks.off.is_empty() && picks.on.is_empty(),
            "the engine already skips it, so there is nothing to remember"
        );

        picks.set(&scope("mono_behaviour/one.assets"), true, false);
        assert_eq!(picks.on, ["mono_behaviour/one.assets"]);

        picks.set(&scope("mono_behaviour/one.assets"), false, false);
        assert!(
            picks.on.is_empty() && picks.off.is_empty(),
            "turning it back off has to leave the list as clean as it started"
        );
    }

    #[test]
    fn turning_everything_on_or_off_replaces_every_choice_below() {
        let mut picks = Picks::default();
        picks.set(&scope("mono_behaviour/one.assets"), true, false);
        picks.set(&scope("text_asset/two.assets"), true, false);

        picks.set(&Scope::default(), true, false);

        assert_eq!(picks.on, [""], "one entry now stands for the whole game");
        assert!(picks.ruling(false).wants("anything/at/all.sheet"));

        picks.set(&Scope::default(), false, false);
        assert!(picks.on.is_empty() && picks.off.is_empty());
    }

    #[test]
    fn a_stored_scope_that_could_climb_out_is_ignored_rather_than_obeyed() {
        let picks = Picks {
            on: vec!["../../etc".to_string()],
            off: Vec::new(),
        };

        assert!(
            !picks.ruling(false).wants("data/Map001.json"),
            "a poisoned entry in a stored project may not switch anything on, or a doctored \
             file could reach outside the game"
        );
    }

    #[test]
    fn a_hand_written_backslash_pick_is_cleared_like_its_slash_twin() {
        let mut picks = Picks {
            on: vec!["data\\Map001.json".to_string()],
            off: Vec::new(),
        };

        assert!(
            picks.ruling(false).wants("data/Map001.json"),
            "the ruling reads the backslash pick, so clearing has to see it too"
        );

        picks.set(&scope("data"), false, false);

        assert!(
            picks.on.is_empty(),
            "turning the folder off has to clear it"
        );
        assert!(!picks.ruling(false).wants("data/Map001.json"));
    }
}
