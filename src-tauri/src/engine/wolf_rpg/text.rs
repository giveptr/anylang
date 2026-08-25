use crate::engine::layout::Boxed;
use crate::engine::wolf_rpg::reached::Reached;
use crate::engine::{filled, same_marks};
use regex::Regex;
use std::sync::LazyLock;

pub const MARKUP_RULES: &str = r"- control codes led by a backslash, e.g. \c[6] \f[20] \v[12] \cself[3] \cdb[0:12:8] \s[5] \font[2] \sp[1] \E \> \^ \.
- ruby, written \r[base,reading]: leave both halves exactly as they are
- tags in angle brackets, e.g. <C> <L> <R>
- language markers in square brackets, e.g. [ENG]: the marker itself, never the words after it
- a row that opens with @ is an engine order, e.g. @1 or @pg wait \cself[10]: copy the whole row through unchanged";

pub const SHAPE_RULES: &str = "- Most items are a whole message box, with its rows separated by \
                               newlines. The box cannot grow: return exactly as many rows as you \
                               were given, in the same order, and keep each row about as short as \
                               the row it replaces. Break the sentence across the rows wherever \
                               it reads best.\n\
                               - An item with no newline in it is one row. Leave it as one row.\n\
                               - Some boxes say the same thing twice, the second copy opening \
                               with a language marker like [ENG]. The marker only says where the \
                               second copy starts; it is not already translated. Translate every \
                               copy into the target language and keep the marker in place.";

pub const RETRY_RULES: &str = "Every backslash control code, every <angle-bracket tag> and every \
                               [UPPERCASE] language marker a source string carries has to appear \
                               in your translation of that string, spelled exactly the same. \
                               Never drop one, and never add one the source does not have. Words \
                               after a language marker are a copy of the message and are \
                               translated like the rest.";

const CODE: &str = r"\\(?:[A-Za-z]+\[(?:[^\[\]\n]|\[[^\]\n]*\])*\]|[A-Za-z]|[.^><|!${}\\])";
const TAG: &str = r"<[[:ascii:]&&[^<>\n]]{1,80}>";
const MARKER: &str = r"\[[A-Z]{2,8}\]";

static RE_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(CODE).expect("CODE is a valid pattern"));

static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(TAG).expect("TAG is a valid pattern"));

static RE_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(MARKER).expect("MARKER is a valid pattern"));

pub static RE_MARK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("{CODE}|{TAG}|{MARKER}")).expect("CODE, TAG and MARKER are valid patterns")
});

pub fn marked(text: &str) -> bool {
    RE_MARK.is_match(text)
}

pub fn has_words(text: &str) -> bool {
    crate::engine::has_words(&RE_MARK, text)
}

pub fn validate(source: &str, translation: &str) -> Result<(), String> {
    filled(translation)?;

    same_marks("control codes", &RE_CODE, source, translation)?;
    same_marks("tags", &RE_TAG, source, translation)?;
    same_marks("language markers", &RE_MARKER, source, translation)?;

    Ok(())
}

const LONGEST_KEY: usize = 20;

fn tokenlike(row: &str) -> bool {
    !row.is_empty()
        && row.len() < LONGEST_KEY
        && row.is_ascii()
        && row
            .chars()
            .all(|one| one.is_ascii_alphanumeric() || one == '_' || one == '-')
}

fn boxed<'a>(was: &'a str, reached: &'a Reached) -> Boxed<'a> {
    Boxed::read(&RE_MARK, was)
        .led_by(move |row| row.starts_with('@') || (tokenlike(row) && reached.builds(row)))
}

pub fn asked(was: &str, reached: &Reached) -> Option<String> {
    boxed(was, reached).asked()
}

pub fn shaped(was: &str, fresh: &str, reached: &Reached) -> String {
    boxed(was, reached).shaped(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::text;

    fn asked(was: &str) -> Option<String> {
        text::asked(was, &Reached::new())
    }

    fn shaped(was: &str, fresh: &str) -> String {
        text::shaped(was, fresh, &Reached::new())
    }

    #[test]
    fn a_key_the_game_builds_a_picture_name_from_keeps_its_own_row() {
        let mut reached = Reached::new();
        reached.ships("hu-n");

        let was = "hu-n\n\u{3048}\u{3063}\u{672c}\u{5f53}\u{306a}\u{306e}";

        assert_eq!(
            text::asked(was, &reached).as_deref(),
            Some("\u{3048}\u{3063}\u{672c}\u{5f53}\u{306a}\u{306e}"),
            "the game reads the first row to pick which portrait to draw"
        );
        assert_eq!(
            text::shaped(
                was,
                "\u{672c}\u{5f53}\u{ff1f}\u{3044}\u{3044}\u{3051}\u{3069}",
                &reached
            ),
            "hu-n\n\u{672c}\u{5f53}\u{ff1f}\u{3044}\u{3044}\u{3051}\u{3069}",
            "however the words are rewrapped, the key stays alone on the row above them"
        );

        assert_eq!(
            text::asked(was, &Reached::new()).as_deref(),
            Some(was),
            "a game that ships no such picture has no key here, only words"
        );
    }

    #[test]
    fn an_order_at_the_head_of_a_box_is_never_offered_for_translation() {
        let was = "@1\n\u{300c}\\cself[8]\u{300d}\u{3092}\u{624b}\u{306b}\u{5165}\u{308c}\u{305f}\u{3002}";

        assert_eq!(
            asked(was).as_deref(),
            Some(
                "\u{300c}\\cself[8]\u{300d}\u{3092}\u{624b}\u{306b}\u{5165}\u{308c}\u{305f}\u{3002}"
            ),
            "the engine reads the first row as an order, so a translator is only shown the words"
        );

        let out = shaped(was, "You picked up \"\\cself[8]\".");
        assert_eq!(out, "@1\nYou picked up \"\\cself[8]\".");
    }

    #[test]
    fn an_order_carrying_an_argument_is_carried_whole() {
        for was in [
            "@pg wait \\cself[10]\n\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}",
            "@standby\n\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}",
        ] {
            let out = shaped(was, "This is a sword");
            let (head, rest) = out.split_once('\n').expect("two rows");

            assert_eq!(
                head,
                was.split_once('\n').expect("two rows").0,
                "an order with a space in it is still an order"
            );
            assert_eq!(rest, "This is a sword");
        }
    }

    #[test]
    fn a_box_holding_no_words_of_its_own_is_left_alone() {
        for was in [
            "@standby\n\\cself[10]\n\\cself[5]",
            "@pg wait \\cself[10]\n\\cself[5]",
        ] {
            assert_eq!(
                asked(was),
                None,
                "{was:?} is an order heading a pair of values, and translating it can only break \
                 it"
            );
            assert_eq!(
                shaped(was, "whatever a model might say"),
                was,
                "nothing was asked for, so nothing may be written back"
            );
        }
    }

    #[test]
    fn a_lone_row_is_words_however_much_it_looks_like_a_key() {
        let mut reached = Reached::new();
        reached.ships("test1");

        assert_eq!(
            text::asked("test", &reached).as_deref(),
            Some("test"),
            "with no rows under it there is no key here, only a word, and a game that ships a \
             picture called something_test1 may not silence it"
        );
    }

    #[test]
    fn a_row_of_bare_control_codes_keeps_its_place_among_the_words() {
        let was = "\u{3042}\u{3042}\n\\s[9]\n\u{3044}\u{3044}";

        assert_eq!(
            asked(was).as_deref(),
            Some("\u{3042}\u{3042}\n\u{3044}\u{3044}"),
            "a row that is only codes is timing, not speech"
        );
        assert_eq!(shaped(was, "First\nSecond"), "First\n\\s[9]\nSecond");
    }

    #[test]
    fn a_box_the_game_wrote_with_crlf_gets_its_crlf_back() {
        let was = "\u{68ee}\u{2026}\r\n\u{9759}\u{304b}\u{306d}\u{2026}";
        let out = shaped(was, "A forest...\nhow quiet...");

        assert_eq!(out, "A forest...\r\nhow quiet...");
    }

    #[test]
    fn a_translation_short_enough_to_drop_a_row_drops_that_rows_line_break_with_it() {
        assert_eq!(
            shaped("\u{3042}\u{3042}\r\n\u{3044}\u{3044}", "Yes"),
            "Yes",
            "the box the author drew had two rows, and an empty second row is a newline the game \
             never wrote, so the row goes and the carriage return that ended it goes with it"
        );
    }

    #[test]
    fn whatever_the_game_wrote_comes_back_byte_for_byte_when_nothing_is_translated() {
        for was in [
            "@1\n\u{300c}\\cself[8]\u{300d}\u{3092}\u{5931}\u{3063}\u{305f}\u{3002}",
            "@standby\n\\cself[10]\n\\cself[5]",
            "\u{68ee}\u{2026}\r\n\u{9759}\u{304b}\u{306d}\u{2026}",
            "\u{3042}\u{3042}\n\\s[9]\n\u{3044}\u{3044}",
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}",
            "\u{3042}\r",
            "<L>\\f[25]\n\u{3042}\u{3042}\r\n\u{3044}\u{3044}",
        ] {
            let said = asked(was).unwrap_or_else(|| was.to_string());

            assert_eq!(
                shaped(was, &said),
                was,
                "handing back exactly what was asked has to rebuild the box unchanged"
            );
        }
    }

    #[test]
    fn a_rule_written_as_prose_reaches_the_model_as_one_line() {
        assert!(!RETRY_RULES.contains('\n'));

        for listed in [MARKUP_RULES, SHAPE_RULES] {
            for line in listed.lines() {
                assert!(
                    line.starts_with("- "),
                    "{line:?} is not a bullet of its own"
                );
            }
        }
    }

    #[test]
    fn a_dropped_control_code_is_refused() {
        let key = "\\c[6]Iron Key\\c[0]";
        assert!(validate(key, "\\c[6]\u{9244}\u{306e}\u{9375}\\c[0]").is_ok());
        assert!(validate(key, "\u{9244}\u{306e}\u{9375}").is_err());

        let lost = "\\E\\f[20]Defeat";
        assert!(
            validate(lost, "\\E\\f[20]\u{6557}\u{5317}").is_ok(),
            "a bare letter code and a numbered one both have to survive"
        );
        assert!(validate(lost, "\\f[20]\u{6557}\u{5317}").is_err());

        assert!(
            validate(
                "\\cself[91] gold",
                "\\cself[19] \u{30b4}\u{30fc}\u{30eb}\u{30c9}"
            )
            .is_err(),
            "the number in a code names which value the engine reads, so changing it reads \
             the wrong one"
        );
        assert!(validate("\\cdb[0:12:8]", "\\cdb[0:12:8]").is_ok());

        assert!(validate(r"Wait\!", "\u{5f85}\u{3066}\\!").is_ok());
        assert!(
            validate(r"Wait\!", "\u{5f85}\u{3066}").is_err(),
            "the punctuation codes rpg_maker already guards are codes here too"
        );
    }

    #[test]
    fn ruby_is_carried_through_untouched_rather_than_translated_apart() {
        let said = "\\r[\u{5e02}\u{677e},\u{3044}\u{3061}\u{307e}\u{3064}]\\r[\u{6a21},\u{3082}]\u{306e}\u{4e0a}";

        assert!(validate(said, &format!("{said} when you walk on it")).is_ok());
        assert!(
            validate(said, "\\r[\u{6a21},\u{3082}]\u{306e}\u{4e0a}").is_err(),
            "the reading sits above the character it belongs to, so losing one loses both"
        );
    }

    #[test]
    fn a_dropped_tag_is_refused() {
        assert!(validate("<C>Title", "<C>\u{984c}\u{540d}").is_ok());
        assert!(validate("<C>Title", "\u{984c}\u{540d}").is_err());
    }

    #[test]
    fn a_nested_database_read_is_carried_whole() {
        let said = r"\cdb[0:\cself[3]:8]";

        assert!(
            validate(
                said,
                "\\cdb[0:\\cself[3]:8]\u{3092}\u{624b}\u{306b}\u{5165}\u{308c}\u{305f}"
            )
            .is_ok()
        );
        assert!(
            validate(said, r"\cdb[0:\cself[9]:8]").is_err(),
            "the inner index names which value the engine reads, so changing it reads the wrong \
             one"
        );
        assert!(
            validate(said, r"\cdb[0:\cself[3]").is_err(),
            "a truncated read loses its tail and the engine parses garbage"
        );
    }

    #[test]
    fn speech_wrapped_in_angle_brackets_is_words_and_not_a_tag() {
        assert!(
            has_words("<\u{3053}\u{3053}\u{306f}\u{5371}\u{306a}\u{3044}>"),
            "a writer can quote a whole line in angle brackets, and calling it a tag would \
             silently drop the line from the sheet"
        );
        assert!(!has_words("<C>"));
    }

    #[test]
    fn a_language_marker_survives_translation_or_the_line_is_refused() {
        let said = "\u{95a2}\u{6240}\u{306e}\u{901a}\u{884c}\n[ENG]Papers, please.";

        assert!(
            validate(said, "Les papiers.\n[ENG]Les papiers, s'il vous plait.").is_ok(),
            "both copies carried into the target language is exactly what is asked for"
        );
        assert!(
            validate(said, "Les papiers, s'il vous plait.").is_err(),
            "a game that splits the line on its marker would lose the second copy"
        );
        assert!(
            validate("Just one language here", "Une seule langue ici").is_ok(),
            "a line with no marker owes none"
        );
    }

    #[test]
    fn a_language_marker_validate_just_enforced_is_never_torn_apart_by_the_box() {
        let laid = shaped("12345678\nx\ny", "abcd[ENG]efghijklmnopqrs");

        assert!(
            laid.contains("[ENG]"),
            "the marker rode through validation whole, so the rows it is broken across have to \
             keep it whole too: {laid:?}"
        );
    }
}
