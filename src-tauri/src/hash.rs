use xxhash_rust::xxh3::Xxh3;

#[derive(Default)]
pub struct Rolling(Xxh3);

impl Rolling {
    pub fn push(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn done(self) -> String {
        format!("{:016x}", self.0.digest())
    }
}

pub fn xxh3(bytes: impl AsRef<[u8]>) -> String {
    let mut hash = Rolling::default();
    hash.push(bytes.as_ref());

    hash.done()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_the_one_every_xxh3_agrees_on() {
        assert_eq!(
            xxh3(""),
            "2d06800538d394c2",
            "this is the published XXH3-64 vector for the empty input, so a build that lands \
             anywhere else is seeding or truncating differently and every mark it writes belongs \
             to a format nobody else reads"
        );

        for (raw, said) in [("a", "e6c632b61e964e1f"), ("foobar", "d78fda63144c5c84")] {
            assert_eq!(
                xxh3(raw),
                said,
                "backups are found by this name, so a hash that quietly changed would strand \
                 every file already put away"
            );
        }
    }

    #[test]
    fn pushing_in_pieces_is_the_same_as_pushing_it_whole() {
        let mut hash = Rolling::default();
        hash.push(b"foo");
        hash.push(b"bar");

        assert_eq!(
            hash.done(),
            xxh3("foobar"),
            "a picture is hashed in the pieces it is read in, so pushing a file through in \
             chunks has to land on the name a single push would have given it"
        );
    }
}
