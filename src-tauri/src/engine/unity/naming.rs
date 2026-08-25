pub fn stem(path: &str) -> String {
    let mut out = String::new();
    let mut deep = 0usize;

    for letter in path.chars() {
        match letter {
            '[' => deep += 1,
            ']' => deep = deep.saturating_sub(1),
            _ if deep == 0 => out.push(letter),
            _ => {}
        }
    }

    out
}

pub fn leaf(path: &str) -> String {
    let stem = stem(path);

    stem.rsplit('.').next().unwrap_or_default().to_string()
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn as_filename(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|letter| match letter {
            '/' | '\\' | ':' | '?' | '*' | '"' | '<' | '>' | '|' => '_',
            _ if letter.is_control() => '_',
            _ => letter,
        })
        .collect();

    let cleaned = cleaned.trim().trim_end_matches('.').trim();

    if cleaned.is_empty() {
        return None;
    }

    let (stem, rest) = cleaned.split_once('.').unwrap_or((cleaned, ""));
    if RESERVED.contains(&stem.to_ascii_uppercase().as_str()) {
        return Some(match rest.is_empty() {
            true => format!("{stem}_"),
            false => format!("{stem}_.{rest}"),
        });
    }

    Some(cleaned.to_string())
}

pub fn named(what: &str, path_id: i64) -> String {
    as_filename(what).unwrap_or_else(|| path_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_name_keeps_its_shape_and_loses_only_the_row_numbers() {
        assert_eq!(stem("dialogCells[0].text"), "dialogCells.text");
        assert_eq!(
            stem("dialogCells[12].charCells[3].name"),
            "dialogCells.charCells.name",
            "two lists deep is still one field"
        );
        assert_eq!(stem("musicPath"), "musicPath");
        assert_eq!(stem("m_Name"), "m_Name");
    }

    #[test]
    fn a_leaf_is_the_last_name_of_the_stem() {
        assert_eq!(leaf("dialogCells[0].text"), "text");
        assert_eq!(leaf("dialogCells[12].charCells[3].name"), "name");
        assert_eq!(leaf("m_Name"), "m_Name");
        assert_eq!(leaf(""), "");
    }

    #[test]
    fn a_device_name_wearing_an_extension_is_still_stepped_around() {
        assert_eq!(
            as_filename("aux.js").as_deref(),
            Some("aux_.js"),
            "Windows reserves the stem whatever follows the dot, so the whole write would fail"
        );
        assert_eq!(as_filename("CON.txt.bak").as_deref(), Some("CON_.txt.bak"));
        assert_eq!(as_filename("auxiliary.js").as_deref(), Some("auxiliary.js"));
    }

    #[test]
    fn a_name_no_filesystem_would_take_still_becomes_a_file() {
        for name in ["?", "a<b>c", "one|two", "say \"hi\"", "star*"] {
            let out = as_filename(name).expect("a filename");
            assert!(
                !out.contains(['?', '*', '"', '<', '>', '|']),
                "{name} became {out}, which Windows would refuse"
            );
        }

        assert_eq!(
            as_filename("mixed_talk\u{1}happy").as_deref(),
            Some("mixed_talk_happy")
        );
        assert_eq!(as_filename("trailing.").as_deref(), Some("trailing"));
        assert_eq!(as_filename("  padded  ").as_deref(), Some("padded"));
    }

    #[test]
    fn a_name_that_would_leave_no_filename_at_all_is_no_name() {
        for name in ["", "   ", ".", "..", "..."] {
            assert_eq!(
                as_filename(name),
                None,
                "{name:?} holds nothing to call the object, and we do not invent one"
            );
        }

        assert_eq!(
            named("", 34_951),
            "34951",
            "an object with no name of its own is called by the only name Unity gave it"
        );
        assert_eq!(
            as_filename("/").as_deref(),
            Some("_"),
            "a separator has something to stand in for it, so it keeps a name"
        );
    }

    #[test]
    fn a_name_a_device_answers_to_is_kept_and_stepped_around() {
        assert_eq!(
            as_filename("NUL").as_deref(),
            Some("NUL_"),
            "the real name is still readable: only the device behind it is stepped around"
        );
        assert_eq!(as_filename("con").as_deref(), Some("con_"));
        assert_eq!(
            as_filename("console").as_deref(),
            Some("console"),
            "only the whole name is reserved, not a word holding it"
        );
    }

    #[test]
    fn a_name_holding_a_separator_still_lands_in_one_file() {
        assert_eq!(as_filename("UI/Main").as_deref(), Some("UI_Main"));
        assert_eq!(
            as_filename("deep\\nested\\name").as_deref(),
            Some("deep_nested_name")
        );
        assert_eq!(
            as_filename("scene051_two_talkers").as_deref(),
            Some("scene051_two_talkers"),
            "an ordinary name is left exactly as it is"
        );
    }
}
