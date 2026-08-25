use crate::engine::unity::cursor;
use crate::engine::unity::cursor::STEP;
use anyhow::{Result, bail};

const HEAD: usize = 4;
const SHORTEST: usize = 2;

pub struct Strand {
    pub at: usize,
    pub text: String,
}

pub fn strands(body: &[u8]) -> Vec<Strand> {
    let mut found = Vec::new();
    let mut at = 0;

    while at + HEAD <= body.len() {
        match reads(body, at) {
            Some((text, past)) => {
                found.push(Strand { at, text });
                at = past;
            }
            None => at += STEP,
        }
    }

    found
}

pub fn splice(body: &[u8], swaps: &[(usize, usize, String)]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len());
    let mut cut = 0;

    for (at, was, fresh) in swaps {
        if !at.is_multiple_of(STEP) {
            bail!("no strand starts at {at}. Unity aligns every one of them to {STEP}");
        }
        if *at < cut {
            bail!("a strand at {at} sits inside the one before it");
        }

        if at + HEAD + was > body.len() {
            bail!("the strand at {at} reaches past the object");
        }

        let held = u32::from_le_bytes(body[*at..at + HEAD].try_into()?) as usize;
        if held != *was {
            bail!("the spot at {at} holds a strand of {held} byte(s), not {was}");
        }

        let past = (at + HEAD + was).next_multiple_of(STEP).min(body.len());

        u32::try_from(fresh.len())
            .map_err(|_| anyhow::anyhow!("a line at {at} is longer than a strand can hold"))?;

        out.extend_from_slice(&body[cut..*at]);
        cursor::put_word(&mut out, fresh);

        cut = past;
    }

    out.extend_from_slice(&body[cut..]);

    Ok(out)
}

fn reads(body: &[u8], at: usize) -> Option<(String, usize)> {
    let (text, past) = cursor::word_at(body, at)?;
    if text.len() < SHORTEST {
        return None;
    }

    if text
        .chars()
        .any(|letter| letter.is_control() && !matches!(letter, '\n' | '\r' | '\t'))
    {
        return None;
    }

    let end = at + HEAD + text.len();
    if body[end..past.min(body.len())]
        .iter()
        .any(|byte| *byte != 0)
    {
        return None;
    }

    Some((text.to_string(), past))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::fake;

    fn texts(body: &[u8]) -> Vec<String> {
        strands(body).into_iter().map(|one| one.text).collect()
    }

    #[test]
    fn every_string_in_a_body_is_found_where_it_sits() {
        let body = fake::strings(&["Wait.", "keep_me", "Is everything ok?"]);
        let found = strands(&body);

        assert_eq!(
            found
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Wait.", "keep_me", "Is everything ok?"]
        );
        assert_eq!(found[0].at, 0);
        assert_eq!(
            found[1].at, 12,
            "a strand is written back at the spot it was found at, so an offset that drifts \
             lays the translation over whatever sits beside it"
        );
    }

    #[test]
    fn a_single_byte_field_is_a_number_rather_than_a_string() {
        let mut body = 3i32.to_le_bytes().to_vec();
        body.extend_from_slice(&1.0f32.to_le_bytes());
        body.extend_from_slice(&1i32.to_le_bytes());
        body.extend_from_slice(&(b'!' as i32).to_le_bytes());
        body.extend_from_slice(&fake::strings(&["Morning."]));

        assert_eq!(
            texts(&body),
            ["Morning."],
            "a table of numbers reads as one byte of text per row, and rewriting a row \
             longer would shift every field behind it"
        );
    }

    #[test]
    fn numbers_that_only_look_like_a_length_are_stepped_over() {
        let mut body = 1i32.to_le_bytes().to_vec();
        body.extend_from_slice(&[0xff, 0, 0, 0]);
        body.extend_from_slice(&fake::strings(&["Wait."]));

        assert_eq!(
            texts(&body),
            ["Wait."],
            "a byte that is not valid text may not be taken for a string"
        );
    }

    #[test]
    fn a_length_reaching_past_the_body_is_refused_instead_of_panicking() {
        let mut body = 4096i32.to_le_bytes().to_vec();
        body.extend_from_slice(b"short");

        assert!(
            texts(&body).is_empty(),
            "a length no body could hold has to come back as nothing rather than unwind out of \
             the middle of a read, or one malformed object costs the container every line in it"
        );
        assert!(texts(&[]).is_empty());
        assert!(texts(&[1, 2]).is_empty());
    }

    #[test]
    fn padding_that_is_not_zero_means_it_was_never_a_string() {
        let mut body = fake::strings(&["abc"]);
        let last = body.len() - 1;
        body[last] = 7;

        assert!(
            texts(&body).is_empty(),
            "Unity zeroes the padding, so anything else is some other kind of field"
        );
    }

    #[test]
    fn putting_nothing_back_gives_the_very_same_bytes() {
        let body = fake::strings(&["one", "two"]);

        assert_eq!(
            splice(&body, &[]).unwrap(),
            body,
            "an export that changed nothing must not touch a single byte, or every run marks \
             the game as changed and the backup grows for no reason"
        );
    }

    #[test]
    fn a_longer_line_keeps_every_strand_after_it_readable() {
        let body = fake::strings(&["Wait.", "keep_me", "Morning."]);
        let found = strands(&body);

        let fresh = splice(
            &body,
            &[(
                found[0].at,
                found[0].text.len(),
                "彼女は首をかたむけた、金の仮面が光っていた。".to_string(),
            )],
        )
        .unwrap();

        assert!(fresh.len() > body.len());
        assert_eq!(
            texts(&fresh),
            [
                "彼女は首をかたむけた、金の仮面が光っていた。",
                "keep_me",
                "Morning."
            ],
            "a translation that grew moves everything behind it, and a strand left at its old \
             spot is a line the game reads as rubbish"
        );
    }

    #[test]
    fn a_shorter_line_closes_the_gap_it_leaves() {
        let body = fake::strings(&["She tilted her head, slowly.", "keep_me"]);
        let found = strands(&body);

        let fresh = splice(
            &body,
            &[(found[0].at, found[0].text.len(), "はい。".to_string())],
        )
        .unwrap();

        assert!(fresh.len() < body.len());
        assert_eq!(
            texts(&fresh),
            ["はい。", "keep_me"],
            "a translation that shrank has to close the gap it left, or the strand after it is \
             read starting from padding"
        );
    }

    #[test]
    fn every_strand_still_starts_on_the_boundary_unity_writes() {
        let body = fake::strings(&["one", "two", "three"]);
        let found = strands(&body);

        let fresh = splice(
            &body,
            &[
                (found[0].at, found[0].text.len(), "ab".to_string()),
                (found[2].at, found[2].text.len(), "bbbbbbbbbb".to_string()),
            ],
        )
        .unwrap();

        for one in strands(&fresh) {
            assert!(
                one.at.is_multiple_of(STEP),
                "a strand landed at {}, which no Unity build would write",
                one.at
            );
        }
        assert_eq!(texts(&fresh), ["ab", "two", "bbbbbbbbbb"]);
    }

    #[test]
    fn a_number_beside_a_line_may_start_looking_like_a_string_and_that_is_fine() {
        let short = "a".repeat(0x19);
        let long = "b".repeat(0x25);

        let mut body = fake::strings(&["I watched her go."]);
        body.extend_from_slice(&1i32.to_le_bytes());
        let at = body.len();
        body.extend_from_slice(&fake::strings(&[&short]));

        assert!(
            texts(&body).contains(&short),
            "0x19 is a control character, so the number in front stays a number"
        );

        let fresh = splice(&body, &[(at, short.len(), long.clone())])
            .expect("the bytes are sound whatever a later scan makes of them");

        assert_eq!(
            reads(&fresh, at).map(|(text, _)| text).as_deref(),
            Some(long.as_str()),
            "0x25 is '%', so a later scan reads the number as a one-letter string and \
             swallows this line: the bytes it was written to are still right"
        );
        assert_eq!(
            &fresh[at - 4..at],
            &1i32.to_le_bytes(),
            "the number in front of it is untouched, which is all the game reads"
        );
    }

    #[test]
    fn a_spot_unity_would_never_align_a_strand_to_is_refused() {
        let body = fake::strings(&["one", "two"]);

        assert!(
            splice(&body, &[(2, 3, "no".to_string())]).is_err(),
            "writing off the boundary would shift every strand after it out of step"
        );
    }

    #[test]
    fn a_spot_that_holds_no_strand_is_refused_rather_than_written_over() {
        let body = fake::strings(&["one"]);

        assert!(
            splice(&body, &[(0, 2, "no".to_string())]).is_err(),
            "the header at 0 says 3 byte(s), so a swap claiming 2 is talking about another spot"
        );
        assert!(
            splice(&body, &[(64, 3, "no".to_string())]).is_err(),
            "a spot past the object holds nothing at all"
        );
    }

    #[test]
    fn strands_given_out_of_order_are_refused() {
        let body = fake::strings(&["one", "two"]);
        let found = strands(&body);

        assert!(
            splice(
                &body,
                &[
                    (found[1].at, found[1].text.len(), "b".to_string()),
                    (found[0].at, found[0].text.len(), "a".to_string()),
                ]
            )
            .is_err(),
            "writing backwards would drop everything between the two"
        );
    }
}
