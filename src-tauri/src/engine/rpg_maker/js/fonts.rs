use crate::backup;
use crate::engine::{Font, Install, fonts as face};
use anyhow::Result;
use std::path::Path;

const DIR: &str = "fonts";

pub fn faces(game_dir: &Path, root: &Path) -> Vec<Font> {
    face::faces(game_dir, &root.join(DIR))
}

pub async fn run(root: &Path, at: &Install<'_>) -> Result<()> {
    let each = faces(at.game_dir, root);
    let held = match at.reverting {
        true => Default::default(),
        false => at.fonts.picked().await?,
    };

    if each.is_empty() {
        if !held.is_empty() {
            at.progress
                .warn(at.doing, "this game ships no font to stand in for");
        }

        return Ok(());
    }

    let (mut stood, mut given) = (0, 0);

    for one in &each {
        let file = at.game_dir.join(&one.at);
        let sent = at.fonts.sent_to(&one.name).and_then(|from| held.get(from));

        match sent {
            Some(body) => {
                backup::replace(at.store, at.game_dir, &file, body.clone()).await?;
                stood += 1;
            }
            None => {
                if backup::put_back(at.store, at.game_dir, &file).await? {
                    given += 1;
                }
            }
        }
    }

    if stood > 0 {
        at.progress.info(
            at.doing,
            &format!("{stood} of the game's {} font(s) stood in for", each.len()),
        );
    }
    if given > 0 {
        at.progress
            .info(at.doing, &format!("{given} font file(s) put back"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;
    use crate::engine::fonts::Fonts;
    use crate::engine::rpg_maker::js::fixture::{put, sandbox};
    use std::fs;

    #[tokio::test]
    async fn each_row_sends_its_own_face_and_a_row_left_empty_changes_nothing() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let staged = sandbox();
        let held = sandbox();

        put(
            root,
            "fonts/gamefont.css",
            "@font-face { src: url(\"body.ttf\"); }",
        );
        put(root, "fonts/body.ttf", "the body font it shipped");
        put(root, "fonts/title.ttf", "the title font it shipped");
        put(root, "fonts/icons.ttf", "the icons it shipped");
        put(held.path(), "sarabun.ttf", "the font a reader picked");
        put(held.path(), "comic.otf", "a second font, for titles");

        let named = |name: &str| held.path().join(name).to_string_lossy().to_string();
        let fonts = Fonts {
            swaps: vec![
                Swap {
                    from: "body.ttf".to_string(),
                    to: named("sarabun.ttf"),
                },
                Swap {
                    from: "title.ttf".to_string(),
                    to: named("comic.otf"),
                },
                Swap {
                    from: "icons.ttf".to_string(),
                    to: String::new(),
                },
            ],
        };

        let told = |reverting| {
            Install::over(root, staged.path(), store.path())
                .sending(&fonts)
                .putting_back(reverting)
        };

        let drawn =
            |name: &str| fs::read_to_string(root.join("fonts").join(name)).expect("a font file");

        run(root, &told(false)).await.expect("the fonts go in");
        assert_eq!(drawn("body.ttf"), "the font a reader picked");
        assert_eq!(drawn("title.ttf"), "a second font, for titles");
        assert_eq!(
            drawn("icons.ttf"),
            "the icons it shipped",
            "a row cleared back to nothing keeps the face the game shipped"
        );
        assert_eq!(
            fs::read_to_string(root.join("fonts/gamefont.css")).unwrap(),
            "@font-face { src: url(\"body.ttf\"); }",
            "the sheet naming the fonts is the game's own and is never rewritten"
        );

        run(root, &told(false)).await.expect("the same fonts again");
        assert_eq!(drawn("body.ttf"), "the font a reader picked");

        run(root, &told(true)).await.expect("the game back");
        assert_eq!(drawn("body.ttf"), "the body font it shipped");
        assert_eq!(drawn("title.ttf"), "the title font it shipped");
        assert_eq!(drawn("icons.ttf"), "the icons it shipped");
    }

    #[test]
    fn the_faces_a_reader_is_offered_are_the_ones_the_game_ships() {
        let at = sandbox();
        let root = at.path();

        put(root, "www/fonts/gamefont.css", "src: url(\"body.ttf\");");
        put(root, "www/fonts/body.ttf", "face");
        put(root, "www/fonts/notes.txt", "not a face");

        let shown: Vec<String> = faces(root, &root.join("www"))
            .into_iter()
            .map(|one| one.at)
            .collect();

        assert_eq!(shown, vec!["www/fonts/body.ttf".to_string()]);
    }

    #[test]
    fn a_game_that_ships_only_web_faces_is_still_offered_them() {
        let at = sandbox();
        let root = at.path();

        put(root, "fonts/mplus-1m-regular.woff", "face");
        put(root, "fonts/mplus-2p-bold-sub.woff2", "another face");

        let shown: Vec<String> = faces(root, root).into_iter().map(|one| one.at).collect();

        assert_eq!(
            shown,
            vec![
                "fonts/mplus-1m-regular.woff".to_string(),
                "fonts/mplus-2p-bold-sub.woff2".to_string(),
            ],
            "mainFontFilename names the file the engine loads, whatever container it is in"
        );
    }
}
