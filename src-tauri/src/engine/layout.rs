use crate::engine::has_words;
use regex::Regex;
use std::mem;

struct Row<'a> {
    text: &'a str,
    cr: bool,
    carried: bool,
}

pub struct Boxed<'a> {
    marks: &'a Regex,
    rows: Vec<Row<'a>>,
}

impl<'a> Boxed<'a> {
    pub fn read(marks: &'a Regex, was: &'a str) -> Self {
        let rows = was
            .split('\n')
            .map(|piece| {
                let (text, cr) = match piece.strip_suffix('\r') {
                    Some(bare) => (bare, true),
                    None => (piece, false),
                };

                Row {
                    text,
                    cr,
                    carried: !has_words(marks, text),
                }
            })
            .collect();

        Self { marks, rows }
    }

    pub fn led_by(mut self, told: impl Fn(&str) -> bool) -> Self {
        let last = self.rows.len().saturating_sub(1);

        for row in self.rows.iter_mut().take(last) {
            if !told(row.text) {
                break;
            }

            row.carried = true;
        }

        self
    }

    fn words(&self) -> Vec<&'a str> {
        self.rows
            .iter()
            .filter(|row| !row.carried)
            .map(|row| row.text)
            .collect()
    }

    pub fn asked(&self) -> Option<String> {
        let said = self.words();

        match said.is_empty() {
            true => None,
            false => Some(said.join("\n")),
        }
    }

    fn laid(&self, translation: &str) -> Vec<String> {
        let words = self.words();
        if words.is_empty() {
            return self.rows.iter().map(|row| row.text.to_string()).collect();
        }

        let mut filled = fit(self.marks, translation, words.len(), &words.join("\n")).into_iter();

        self.rows
            .iter()
            .map(|row| match row.carried {
                true => row.text.to_string(),
                false => filled.next().unwrap_or_default(),
            })
            .collect()
    }

    pub fn laid_over(&self, translation: &str, slots: usize) -> Vec<String> {
        let mut rows = self.laid(translation);

        while rows.len() > slots {
            let tail = rows.pop().unwrap_or_default();

            match rows.last_mut() {
                Some(last) if !tail.is_empty() => {
                    last.push('\n');
                    last.push_str(&tail);
                }
                _ => {}
            }
        }

        rows.resize(slots, String::new());

        rows
    }

    pub fn shaped(&self, translation: &str) -> String {
        let mut laid = self.laid(translation);
        let whole = laid.len();

        while laid
            .len()
            .checked_sub(1)
            .is_some_and(|last| laid[last].is_empty() && !self.rows[last].carried)
        {
            laid.pop();
        }

        let last = laid.len().saturating_sub(1);
        let dropped = whole - laid.len();
        let mut said = String::new();

        for (which, text) in laid.iter().enumerate() {
            said.push_str(text);

            if self.rows[which].cr && !(which == last && dropped > 0) {
                said.push('\r');
            }
            if which != last {
                said.push('\n');
            }
        }

        said
    }
}

fn measure(marks: &Regex, text: &str) -> usize {
    let mut total = 0;
    let mut at = 0;

    for found in marks.find_iter(text) {
        total += text[at..found.start()].chars().count();
        at = found.end();
    }

    total + text[at..].chars().count()
}

fn room(marks: &Regex, source: &str) -> usize {
    source
        .split('\n')
        .map(|row| measure(marks, row))
        .max()
        .unwrap_or_default()
        .max(8)
}

fn fit(marks: &Regex, translation: &str, slots: usize, source: &str) -> Vec<String> {
    if slots == 0 {
        return Vec::new();
    }

    let width = room(marks, source);
    let given: Vec<&str> = translation.split('\n').collect();

    let mut rows = if given.len() <= slots && given.iter().all(|row| measure(marks, row) <= width) {
        given.into_iter().map(str::to_string).collect()
    } else {
        reflow(marks, &given.join(" "), slots, width)
    };

    rows.resize(slots, String::new());

    rows
}

fn reflow(marks: &Regex, text: &str, slots: usize, width: usize) -> Vec<String> {
    let mut wide = measure(marks, text).div_ceil(slots).max(width);

    loop {
        let rows = wrap(marks, text, wide);
        if rows.len() <= slots {
            return rows;
        }

        wide += 1;
    }
}

fn wrap(marks: &Regex, text: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();

    for word in text.split_whitespace() {
        for piece in chop(marks, word, width) {
            if row.is_empty() {
                row = piece;
                continue;
            }

            if measure(marks, &row) + 1 + measure(marks, &piece) <= width {
                row.push(' ');
                row.push_str(&piece);
            } else {
                rows.push(mem::replace(&mut row, piece));
            }
        }
    }

    if !row.is_empty() {
        rows.push(row);
    }

    rows
}

fn chop(marks: &Regex, word: &str, width: usize) -> Vec<String> {
    if measure(marks, word) <= width {
        return vec![word.to_string()];
    }

    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut used = 0;
    let mut at = 0;

    while at < word.len() {
        let mark = marks.find_at(word, at).filter(|found| found.start() == at);

        let (take, cost) = match mark {
            Some(found) => (found.as_str(), 0),
            None => {
                let step = word[at..].chars().next().expect("a char at a boundary");
                (&word[at..at + step.len_utf8()], 1)
            }
        };

        if used + cost > width && !piece.is_empty() {
            pieces.push(mem::take(&mut piece));
            used = 0;
        }

        piece.push_str(take);
        used += cost;
        at += take.len();
    }

    if !piece.is_empty() {
        pieces.push(piece);
    }

    pieces
}

#[cfg(test)]
mod tests {
    use crate::engine::layout;
    use regex::Regex;
    use std::sync::LazyLock;
    static RE_MARK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\\(?:[A-Za-z]+\[[^\]]*\]|[A-Za-z]|[.|!^<>{}$\\])|<[^<>\n]{1,80}>")
            .expect("the pattern these tests measure against")
    });

    fn fit(translation: &str, slots: usize, source: &str) -> Vec<String> {
        layout::fit(&RE_MARK, translation, slots, source)
    }

    fn measure(text: &str) -> usize {
        layout::measure(&RE_MARK, text)
    }

    fn chop(word: &str, width: usize) -> Vec<String> {
        layout::chop(&RE_MARK, word, width)
    }

    fn boxed(was: &str) -> layout::Boxed<'_> {
        layout::Boxed::read(&RE_MARK, was)
    }

    #[test]
    fn a_plugin_tag_on_its_own_row_is_neither_asked_about_nor_written_over() {
        let was = "<center>\nYou picked the newest update.\nIt drops you straight into it.";

        assert_eq!(
            boxed(was).asked().as_deref(),
            Some("You picked the newest update.\nIt drops you straight into it."),
            "a row of nothing but tags tells the plugin how to draw the box"
        );

        let laid = boxed(was).laid(
            "The model answered with one very long row that has to be rewrapped to fit the box \
             it is going into, which is exactly when rows get merged.",
        );

        assert_eq!(laid.len(), 3, "the box still has the rows it started with");
        assert_eq!(
            laid[0], "<center>",
            "and the tag keeps its own row however the words are rewrapped"
        );
        assert!(!laid[1].contains("<center>"));
    }

    #[test]
    fn a_row_naming_who_speaks_is_carried_rather_than_translated() {
        let was = "\\n[1]:\nI should put some money aside for her.";

        assert_eq!(
            boxed(was).asked().as_deref(),
            Some("I should put some money aside for her."),
        );
        assert_eq!(
            boxed(was).laid("\u{5c11}\u{3057}\u{8caf}\u{3081}\u{3088}\u{3046}\u{304b}")[0],
            "\\n[1]:"
        );
    }

    #[test]
    fn a_box_of_nothing_but_tags_is_handed_back_exactly_as_it_came() {
        let was = "<center>\n<b></b>";

        assert_eq!(boxed(was).asked(), None);
        assert_eq!(boxed(was).shaped("anything at all"), was);
        assert_eq!(boxed(was).laid("anything at all"), ["<center>", "<b></b>"]);
    }

    #[test]
    fn every_row_a_box_holds_comes_back_when_the_words_are_handed_straight_back() {
        for was in [
            "<center>\nOne row.\nAnother row.",
            "\\n[1]:\nJust the one.",
            "plain\nrows\nonly",
            "trailing blank\n",
            "<center>\n<b></b>",
        ] {
            let said = boxed(was).asked().unwrap_or_else(|| was.to_string());

            assert_eq!(boxed(was).shaped(&said), was, "{was:?} did not survive");
        }
    }

    #[test]
    fn no_line_is_ever_dropped_when_the_translation_has_more_of_them() {
        let kept = fit("one\ntwo\nthree\nfour", 2, "aaaa\nbbbb");

        assert_eq!(kept.len(), 2, "the game only has room for two rows");
        let whole = kept.join(" ");
        for word in ["one", "two", "three", "four"] {
            assert!(whole.contains(word), "{word} was lost");
        }
    }

    #[test]
    fn a_short_translation_blanks_the_rows_it_no_longer_needs() {
        assert_eq!(
            fit("all of it", 3, "the row that sets the width\nsecond\nthird"),
            vec!["all of it", "", ""]
        );
    }

    #[test]
    fn a_row_too_wide_for_the_box_is_rewrapped_to_fit() {
        let source = "The box is exactly this wide, no more.\nSecond row here.";
        let widest = 37;

        let rows = fit(
            "The model put everything on a single long row instead.",
            2,
            source,
        );

        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(
                row.chars().count() <= widest,
                "{row:?} is wider than the message box"
            );
        }
    }

    #[test]
    fn japanese_without_spaces_is_still_broken_across_the_rows() {
        let source = "This row sets the width.\nSecond row.";
        let answer = "ここはとても長い日本語の文章で、箱の幅を超えてしまうはずです。";

        let rows = fit(answer, 2, source);

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().all(|row| row.chars().count() <= 24),
            "{rows:?} still overflows"
        );
        assert_eq!(
            rows.concat(),
            answer,
            "not a character may be lost or a space slipped in"
        );
    }

    #[test]
    fn a_control_code_is_never_split_across_rows() {
        let word = r"aaaa\C[6]bbbb";
        let rows = chop(word, 4);

        assert_eq!(rows.concat(), word, "chopping may not lose a letter");
        assert!(
            rows.iter().all(|row| measure(row) <= 4),
            "every row has to fit the box: {rows:?}"
        );
        assert!(
            rows.iter()
                .all(|row| !row.contains('\\') || row.contains(r"\C[6]")),
            "a control code split across rows corrupts the message box: {rows:?}"
        );
    }
}
