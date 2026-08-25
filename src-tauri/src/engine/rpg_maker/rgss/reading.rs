use crate::engine::rpg_maker::rgss::source::Source;
use crate::engine::rpg_maker::rgss::{data, scripts, tes};
use crate::engine::sheet;
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};

pub struct Reading {
    pub keys: Option<String>,
    pub known: HashSet<String>,
}

pub enum Held {
    Scenario(Vec<u8>),
    Scripts,
    Sheet,
    Locked,
}

impl Reading {
    pub async fn of(held: &Source) -> Result<Self> {
        let sheets = held.sheets();
        let known: HashSet<String> = held.known();

        let keys = match sheets.iter().find(|(_, one)| one == scripts::NAME) {
            Some((which, _)) => tes::keys_of(&held.body(*which).await?),
            None => None,
        };

        Ok(Self { keys, known })
    }

    pub fn held(&self, named: &str, body: &[u8]) -> Held {
        if let Some(inner) = self
            .keys
            .as_deref()
            .and_then(|keys| tes::decoded(body, keys))
        {
            return Held::Scenario(inner);
        }

        if tes::looks_locked(body) {
            return Held::Locked;
        }

        match named == scripts::NAME {
            true => Held::Scripts,
            false => Held::Sheet,
        }
    }

    pub fn lines_of(&self, held: &Held, body: &[u8]) -> Result<Vec<sheet::Line>, String> {
        match held {
            Held::Scenario(inner) => data::lines_of(inner),
            Held::Scripts => scripts::lines_of(body, &self.known),
            Held::Sheet => data::lines_of(body),
            Held::Locked => Ok(Vec::new()),
        }
    }

    pub fn spliced(
        &self,
        held: &Held,
        body: &[u8],
        said: &BTreeMap<String, String>,
    ) -> Result<(Vec<u8>, u32), String> {
        match held {
            Held::Scenario(inner) => {
                let keys = self
                    .keys
                    .as_deref()
                    .ok_or("the scenario key went missing")?;
                let (fresh, written) = data::spliced(inner, said)?;

                Ok((tes::encoded(&fresh, keys)?, written))
            }
            Held::Scripts => scripts::spliced(body, said),
            Held::Sheet => data::spliced(body, said),
            Held::Locked => Ok((body.to_vec(), 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_sheet_is_read_as_itself_and_a_locked_one_is_named_as_locked() {
        let reading = Reading {
            keys: None,
            known: HashSet::new(),
        };

        assert!(matches!(
            reading.held("Map001", &[4, 8, b'[', 6, b'i', 6]),
            Held::Sheet
        ));
        assert!(matches!(
            reading.held(scripts::NAME, &[4, 8, b'[', 6, b'i', 6]),
            Held::Scripts
        ));

        let locked = tes::encoded(&[4, 8, b'0'], "b61a0f29").expect("it packs");
        assert!(
            matches!(reading.held("main", &locked), Held::Locked),
            "without the key it is still plainly a container"
        );

        let (same, written) = reading
            .spliced(&Held::Locked, &locked, &BTreeMap::new())
            .expect("a locked sheet writes nothing");
        assert_eq!((same, written), (locked, 0));
    }
}
