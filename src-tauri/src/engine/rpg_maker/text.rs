use crate::engine::{filled, same_marks, symbolic};
use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;

pub const MARKUP_RULES: &str = r"- control codes led by a backslash, e.g. \V[3] \N[1] \P[2] \C[6] \I[64] \G \$ \. \| \! \{ \}
- tags in angle brackets, e.g. <center> <WordWrap> <br> <ColorLock> <Choice ...>
- placeholders, e.g. %1 %2 %s
- switch and variable ids a plugin reads, e.g. s[133] v[14]
- a choice condition a plugin reads, copied whole down to the comparison, e.g. if(s[133]) en(v[14] >= 1) dis(v[7] < 3)";

pub const SHAPE_RULES: &str = "- Some items arrive as several lines joined by newlines. The game \
                               reads each line on its own and the item cannot grow: return exactly \
                               as many lines as you were given, in the same order, and keep each \
                               line about as short as the line it replaces. Break the sentence \
                               across the lines wherever it reads best.\n\
                               - Most items are a single line with no newline in them. Leave those \
                               as one line.";

pub const RETRY_RULES: &str = "Every backslash control code, every <angle-bracket tag>, every \
                               placeholder such as %1 or %s, every switch or variable id such as \
                               s[133] or v[14] and every condition such as if(s[133]) or \
                               en(v[14] >= 1) a source string carries has to appear in your \
                               translation of that string, spelled exactly the same, comparison \
                               and all. Never drop one, and never add one the source does not \
                               have.";

const CODE: &str = r"\\(?:[A-Za-z]+\[[^\]]*\]|[A-Za-z]|[.|!^<>{}$\\])";
const TAG: &str = r"<[^<>\n]{1,80}>";
const PLACEHOLDER: &str = r"%(?:\d|[sd]\b)";

static RE_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(CODE).expect("CODE is a valid pattern"));

static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(TAG).expect("TAG is a valid pattern"));

static RE_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(PLACEHOLDER).expect("PLACEHOLDER is a valid pattern"));

pub static RE_MARK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("{CODE}|{TAG}")).expect("CODE and TAG are valid patterns")
});

const REFERENCE: &str = r"\b[A-Za-z]\[\d+\]";
const CONDITION: &str = r"\b(?:if|en|dis|show|hide)\((?:[^()\n]|\([^()\n]*\))*\)";

static RE_REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(REFERENCE).expect("REFERENCE is a valid pattern"));

static RE_CONDITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(CONDITION).expect("CONDITION is a valid pattern"));

const GLOBAL: &str = r"\$(?:game|data)[A-Z]";
const COMMENT: &str = r"^\s*//";
const OPERATOR: &str = r"===|!==|&&|\|\|";
const BLOCK: &str = r"^\s*(?:if|for|while)\s*\(.*\)\s*\{\s*$";
const DECLARED: &str = r"^\s*(?:function|var|const)\s+[A-Za-z_$]";
const CALL: &str = r"[\p{L}_$][\p{L}\p{N}_$]{2,}\(\s*(?:\d|\))";
const MEMBER: &str = r"[A-Za-z_$][A-Za-z0-9_$]{2,}\.[a-z][A-Za-z0-9_$]*";
const ASSIGNED: &str = r"[-+*/%]=[^=]";
const STEPPED: &str = r"[A-Za-z_$\])](?:\+\+|--)|(?:\+\+|--)[A-Za-z_$(]";
const REMAINDER: &str = r"[)\]]\s*%";

const NUMBER: &str = r"\d+(?:\.\d+)?";
const FIELD: &str = r"[A-Za-z_$][A-Za-z0-9_$]*\.[A-Za-z_$][A-Za-z0-9_$]*";

static RE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "(?m){GLOBAL}|{COMMENT}|{OPERATOR}|{BLOCK}|{DECLARED}|{CALL}|{MEMBER}|{ASSIGNED}|\
         {STEPPED}|{REMAINDER}"
    ))
    .expect("every script pattern is a valid one")
});

static RE_MATHS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?:{NUMBER}|{FIELD}|[)\]])\s*[-+*/]|[-+*/]\s*(?:{NUMBER}|{FIELD}|\()"
    ))
    .expect("every maths pattern is a valid one")
});

pub fn symbolic_line(text: &str) -> bool {
    RE_MARK.replace_all(text, "") == text && symbolic(text)
}

pub fn looks_like_code(text: &str) -> bool {
    RE_SCRIPT.is_match(text) || arithmetic(text)
}

fn arithmetic(text: &str) -> bool {
    let line = text.trim();

    line.is_ascii() && !line.contains('\n') && RE_MATHS.is_match(line) && !two_words_running(line)
}

fn two_words_running(line: &str) -> bool {
    let words = line.split_whitespace();

    words
        .clone()
        .zip(words.skip(1))
        .any(|(before, after)| a_word(before) && a_word(after))
}

fn a_word(token: &str) -> bool {
    token.len() > 1 && token.chars().all(|one| one.is_ascii_alphabetic())
}

pub fn listed_line(text: &str) -> bool {
    symbolic_line(text) || looks_like_code(text)
}

pub fn unmarked(text: &str) -> Cow<'_, str> {
    RE_MARK.replace_all(text, "")
}

pub fn has_words(text: &str) -> bool {
    crate::engine::has_words(&RE_MARK, text)
}

pub fn validate(source: &str, translation: &str) -> Result<(), String> {
    filled(translation)?;

    same_marks("control codes", &RE_CODE, source, translation)?;
    same_marks("tags", &RE_TAG, source, translation)?;
    same_marks("placeholders", &RE_PLACEHOLDER, source, translation)?;
    same_marks(
        "switch and variable ids",
        &RE_REFERENCE,
        source,
        translation,
    )?;
    same_marks("choice conditions", &RE_CONDITION, source, translation)?;

    for stray in ['<', '>'] {
        if translation.matches(stray).count() > source.matches(stray).count() {
            return Err(format!("angle brackets: unexpected {stray}"));
        }
    }

    Ok(())
}

#[cfg(test)]
pub const TALK: &[&str] = &["(Hm...)", "SPAS-12", "Slime*2", "Nightshade", "Castle Town"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_written_as_prose_reaches_the_model_as_one_line() {
        assert!(
            !RETRY_RULES.contains('\n'),
            "retry rules are one paragraph, so a break in them is only this file's wrapping and \
             the model reads it as a break the author never meant"
        );

        for listed in [MARKUP_RULES, SHAPE_RULES] {
            for line in listed.lines() {
                assert!(
                    line.starts_with("- "),
                    "every line of a rule list is its own bullet, so a wrapped one reads as a \
                     rule of its own: {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_plugin_setting_holding_a_script_is_never_offered_as_a_line_to_translate() {
        for said in [
            "// Undersea (First time)\nif ($gameMap.mapId() === 90 && $gameVariables.value(197) \
             === 4) {\n    $gameTemp.reserveCommonEvent(28);\n}",
            "$gameSwitches.setValue(12, true);",
            "if (a.hp < 100) {\n  b.gainHp(20);\n}",
            "// nothing here yet",
            "function opening() {\n  return 1;\n}",
            "rgba(32, 32, 32, 0.5)",
            "(Graphics.height / 2) - 312",
            "SceneManager.push(Scene_Menu)",
            "\u{6642}\u{9593}\u{5224}\u{5b9a}();",
            "if (TouchInput.wheelY !== 0) {\n    \u{79fb}\u{52d5}\u{30b3}\u{30de}\u{30f3}\u{30c9}\u{30db}\u{30a4}\u{30fc}\u{30eb}();\n}",
        ] {
            assert!(
                looks_like_code(said),
                "a plugin author can leave code in a setting the plugin never marked as code, \
                 and handing it to a model comes back as a broken game: {said:?}"
            );
        }

        for said in [
            "Let me think about it for a while.",
            "The value of friendship (and a good sword) is beyond measure.",
            "Wait! Don't go in there. The bridge is out.",
            "if(s[133]) You already have the key.",
            "100% cotton, or so the merchant claims.",
            "Return to the village and speak with the elder.",
            "Sell your item(s) here for a fair price.",
            "Ask Mr.Smith about the missing crate.",
            "Rest here to recover HP and MP (costs 10 gold).",
        ] {
            assert!(
                !looks_like_code(said),
                "this is a line a person wrote for a player to read, and calling it code would \
                 quietly drop it from the translation: {said:?}"
            );
        }
    }

    #[test]
    fn a_formula_written_with_short_names_is_still_code() {
        for said in [
            "a.y - a.height / 2",
            "b.width / 3",
            "sy + (b.y - b.height/2 - sy) * (ex - sx) / (b.x - sx)",
            "(r/repeat * Math.PI*2 - Math.PI/2)",
            "100 * (1 - r/repeat)",
            "allRangeY * 1.5 * t/arrival",
            "defaultX + 250 * mirroring",
            "100 + textSize * 10",
            "dataA[no-1].list[r].arcX * -1",
            "b.y - (position == 0 ? b.height : position == 1 ? b.height/2 : 0)",
        ] {
            assert!(
                looks_like_code(said),
                "a plugin fills a coordinate in by hand and names the fighters a and b, so \
                 reading the member the name hangs off of never sees it: {said:?}"
            );
        }
    }

    #[test]
    fn a_name_shaped_like_a_sum_is_listed_rather_than_sent_off() {
        for named in ["SPAS-12", "Slime*2"] {
            assert!(
                looks_like_code(named),
                "a name a player reads can be spelled the way a formula is, and telling the two \
                 apart by which letters are capitals bets on one game's habits. A listed line is \
                 still on the reader's screen to settle by hand, while a formula handed to a \
                 model comes back as a battle that no longer draws: {named:?}"
            );
        }
    }

    #[test]
    fn a_line_of_talk_carrying_a_sign_still_reaches_the_model() {
        for said in [
            "Hey - stop!",
            "It's 50% off!",
            "Sorry, I can't - it's too far.",
            "Yes/No",
            "Fire + Ice",
            "You have 5 + 3 apples",
        ] {
            assert!(
                !looks_like_code(said),
                "a dash, a slash and a percent are punctuation far more often than they are \
                 arithmetic, so a sign on its own says nothing. What tells a formula apart is \
                 what stands beside the sign: a number, a field read off a name, or a bracketed \
                 group. None of those is here, so this is a line the player reads: {said:?}"
            );
        }
    }

    #[test]
    fn a_line_the_engine_runs_rather_than_reads_is_still_kept_off_the_bill() {
        for held in [
            "x += 1",
            "$gameVariables._data[7] -= 2",
            "i++",
            "--count",
            "v[1] % 3",
            "this._index %= this.maxItems()",
        ] {
            assert!(
                looks_like_code(held),
                "compound assignment, stepping a counter and taking a remainder off a bracketed \
                 group are spellings only a script uses, so nothing here is a line the player \
                 reads and paying a model to translate it buys nothing: {held:?}"
            );
        }
    }

    #[test]
    fn a_dropped_control_code_is_refused() {
        assert!(validate(r"Hello \N[1]!", r"こんにちは、\N[1]!").is_ok());
        assert!(validate(r"Hello \N[1]!", "こんにちは!").is_err());
        assert!(validate(r"You have \V[12] gold", r"\V[12] ゴールド持っている").is_ok());
        assert!(validate(r"You have \V[12] gold", r"\V[21] ゴールド持っている").is_err());
    }

    #[test]
    fn a_bare_code_without_brackets_still_counts() {
        assert!(validate(r"Costs 100\G.", r"100\Gです。").is_ok());
        assert!(validate(r"Costs 100\G.", "100です。").is_err());
    }

    #[test]
    fn a_dropped_tag_is_refused() {
        assert!(validate("<center>Title</center>", "<center>題名</center>").is_ok());
        assert!(validate("<center>Title</center>", "題名").is_err());
    }

    #[test]
    fn a_dropped_placeholder_is_refused() {
        assert!(validate("%1 took %2 damage!", "%1は%2のダメージを受けた!").is_ok());
        assert!(validate("%1 took %2 damage!", "%1はダメージを受けた!").is_err());
        assert!(validate("%1 took %2 damage!", "%2は%1のダメージを受けた!").is_ok());
    }

    #[test]
    fn a_ruby_style_placeholder_is_watched_like_a_numbered_one() {
        assert!(validate("Obtained %s!", "%s を入手!").is_ok());
        assert!(validate("Obtained %s!", "入手!").is_err());
        assert!(
            validate("50%discount today", "本日50%discount").is_ok(),
            "a percent sign glued to a word is prose, not a placeholder"
        );
    }

    #[test]
    fn an_angle_bracket_the_source_never_wrote_is_refused() {
        assert!(
            validate("Empire Army", "帝国 > 軍").is_err(),
            "a note value is written back raw, and a stray bracket in it ends the tag early"
        );
        assert!(validate("A > B", "甲 > 乙").is_ok());
        assert!(validate("<center>Title</center>", "<center>題名</center>").is_ok());
    }

    #[test]
    fn a_switch_id_a_plugin_reads_has_to_come_back_untouched() {
        assert!(validate("ON if(s[1201])", "オン if(s[1201])").is_ok());
        assert!(
            validate("ON if(s[1201])", "オン").is_err(),
            "a conditional choice loses its condition and the choice shows up always"
        );
        assert!(validate("Gacha en(v[13] >= 1)", "Gacha en(v[31] >= 1)").is_err());
    }

    #[test]
    fn the_comparison_a_condition_gates_on_is_part_of_the_condition() {
        let gate = "Gacha en(v[13] >= 1)";

        assert!(validate(gate, "ガチャ en(v[13] >= 1)").is_ok());
        assert!(
            validate(gate, "ガチャ en(v[13] > 1)").is_err(),
            "the id came back untouched and only the comparison moved, so the choice greys out \
             one draw later than the author wrote"
        );
        assert!(validate(gate, "ガチャ en(v[13] >= 2)").is_err());
    }

    #[test]
    fn a_condition_naming_no_id_at_all_is_still_carried_whole() {
        let call = "Buy if($gameSwitches.value(3))";

        assert!(validate(call, "買う if($gameSwitches.value(3))").is_ok());
        assert!(
            validate(call, "買う if($gameSwitches.value(4))").is_err(),
            "the test holds no s[n] or v[n] for the id check to compare, so the condition itself \
             is the only thing standing between the player and a choice meant to be hidden"
        );
    }

    #[test]
    fn every_word_a_plugin_gates_a_choice_with_is_watched() {
        for verb in ["if", "en", "dis", "show", "hide"] {
            let gate = format!("Enter {verb}(s[4])");

            assert!(validate(&gate, &format!("入る {verb}(s[4])")).is_ok());
            assert!(
                validate(&gate, "入る").is_err(),
                "{verb} decides whether the player ever sees the choice, and dropping it hands \
                 them one the author locked away"
            );
        }
    }

    #[test]
    fn a_name_a_plugin_looks_up_is_told_apart_from_a_line_of_talk() {
        for reference in [
            "event:4",
            "ripple_walk",
            "00makai_16",
            "a=1;b=2;",
            "clearWindowColor",
            "hideBalloon",
            "IsPhoneOwned",
            "AngelinaDayWait",
            "---Drops---",
        ] {
            assert!(symbolic(reference), "{reference} names something");
        }
        for talk in TALK {
            assert!(!symbolic(talk), "{talk} is text a player reads");
        }
        assert!(
            !symbolic("こうげき"),
            "an identifier a plugin compares is always ASCII"
        );
        for lone in ["A", "b", "1", "-"] {
            assert!(
                symbolic(lone),
                "{lone} is one ascii character, and there is nothing in one letter to translate"
            );
        }
        assert!(
            !symbolic("力"),
            "one character is a whole word in a language that writes this way"
        );
    }

    #[test]
    fn plain_text_needs_no_markup_at_all() {
        assert!(validate("Just words.", "ただの文字。").is_ok());
        assert!(validate("Just words.", "   ").is_err());
    }

    #[test]
    fn a_value_made_only_of_control_codes_is_not_worth_a_request() {
        assert!(!has_words(r"\v[45]"));
        assert!(!has_words(r"\c[14]\v[51]"));
        assert!(!has_words("<center>"));
        assert!(!has_words("100"), "a bare number has nothing to translate");
        assert!(!has_words("   "));
    }

    #[test]
    fn words_are_recognised_whatever_the_writing_system() {
        assert!(has_words("Potion"));
        assert!(has_words(r"\c[6]Iron Key\c[0]"));
        assert!(
            has_words("こんにちは"),
            "the source language is not always Latin"
        );
        assert!(has_words("Здравствуйте"));
        assert!(has_words("Épée"));
    }
}
