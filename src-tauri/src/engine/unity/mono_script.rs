use crate::engine::unity::serial::{Container, Object};
use crate::engine::unity::{dotnet, serial};
use std::collections::BTreeMap;

const SCRIPT_AT: usize = 16;

const CLASS: &str = "m_ClassName";
const SPACE: &str = "m_Namespace";

#[derive(Default)]
pub struct Names {
    classes: BTreeMap<(String, i64), String>,
}

impl Names {
    pub fn learn(&mut self, container: &Container) {
        for object in &container.objects {
            if object.class_id != serial::MONO_SCRIPT {
                continue;
            }

            if let Some(name) = class_in(object) {
                self.classes
                    .insert((file_key(&container.name), object.path_id), name);
            }
        }
    }

    pub fn of(&self, container: &Container, object: &Object) -> Option<&str> {
        self.told(&points_to(container, object)?)
    }

    pub fn told(&self, script: &(String, i64)) -> Option<&str> {
        self.classes.get(script).map(String::as_str)
    }
}

pub fn points_to(container: &Container, object: &Object) -> Option<(String, i64)> {
    let (file, path_id) = points_at(&object.body().ok()?)?;

    Some((owner_of(container, file)?, path_id))
}

pub fn owner_of(container: &Container, file: i32) -> Option<String> {
    match file {
        0 => Some(file_key(&container.name)),
        _ => Some(file_key(
            container.externals.get(usize::try_from(file).ok()? - 1)?,
        )),
    }
}

pub fn file_key(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase()
}

fn points_at(body: &[u8]) -> Option<(i32, i64)> {
    let raw = body.get(SCRIPT_AT..SCRIPT_AT + 12)?;

    Some((
        i32::from_le_bytes(raw[..4].try_into().ok()?),
        i64::from_le_bytes(raw[4..].try_into().ok()?),
    ))
}

fn class_in(object: &Object) -> Option<String> {
    let value = object.value()?;
    let name = value.field(CLASS)?.text()?;
    let space = value.field(SPACE)?.text()?;

    (!name.is_empty()).then(|| dotnet::full_name(&space, &name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::fake;

    fn named_as(path_id: i64, held: Object) -> Object {
        Object::forged(
            serial::MONO_SCRIPT,
            path_id,
            held.body().expect("its body").into_owned(),
        )
    }

    fn a_script(name: &str, class: &str) -> Object {
        in_space(name, "", class)
    }

    fn in_space(name: &str, space: &str, class: &str) -> Object {
        Object::forged(
            serial::MONO_SCRIPT,
            11,
            fake::drawing(
                serial::MONO_SCRIPT,
                vec![
                    ("m_Name", fake::text(name)),
                    ("m_ClassName", fake::text(class)),
                    ("m_Namespace", fake::text(space)),
                ],
            ),
        )
    }

    fn a_behaviour(file: i32, path_id: i64) -> Vec<u8> {
        let mut out = vec![0u8; SCRIPT_AT];
        out.extend_from_slice(&file.to_le_bytes());
        out.extend_from_slice(&path_id.to_le_bytes());
        out.extend_from_slice(&fake::strings(&[""]));

        out
    }

    #[test]
    fn a_script_gives_up_the_class_it_stands_for() {
        assert_eq!(
            class_in(&a_script("SceneImageLoader", "SceneImageLoader")).as_deref(),
            Some("SceneImageLoader")
        );
        assert_eq!(
            class_in(&in_space("", "TMPro", "TMP_FontAsset")).as_deref(),
            Some("TMPro.TMP_FontAsset"),
            "the namespace is what tells two same-named classes apart"
        );
        assert!(
            class_in(&Object::forged(serial::MONO_SCRIPT, 11, Vec::new())).is_none(),
            "a body with nothing in it names no class"
        );
        assert!(
            class_in(&a_script("only a name", "")).is_none(),
            "a script that names no class of its own is one no behaviour can be filed under"
        );
    }

    #[test]
    fn a_behaviour_is_named_by_the_script_it_points_at() {
        let mut names = Names::default();
        names.learn(&fake::container(
            "globalgamemanagers.assets",
            vec![named_as(900, a_script("", "SceneHandler"))],
            &[],
        ));

        let scene = fake::container(
            "sharedassets0.assets",
            vec![Object::forged(
                serial::MONO_BEHAVIOUR,
                7,
                a_behaviour(1, 900),
            )],
            &["globalgamemanagers.assets"],
        );

        assert_eq!(
            names.of(&scene, &scene.objects[0]),
            Some("SceneHandler"),
            "a behaviour holds no class name of its own, only a pointer into another \
             container's scripts, so leaving that pointer unfollowed leaves every field in the \
             object unreadable"
        );
    }

    #[test]
    fn a_script_living_in_the_same_container_is_found_too() {
        let mut names = Names::default();
        let one = fake::container(
            "level0",
            vec![
                named_as(900, a_script("", "UILabel")),
                Object::forged(serial::MONO_BEHAVIOUR, 7, a_behaviour(0, 900)),
            ],
            &[],
        );
        names.learn(&one);

        assert_eq!(
            names.of(&one, &one.objects[1]),
            Some("UILabel"),
            "file zero is the container doing the asking, not the first one it lists, so \
             reading it as a neighbour looks the script up in the wrong place"
        );
    }

    #[test]
    fn a_bundle_names_itself_the_way_its_neighbours_point_at_it() {
        let mut names = Names::default();
        names.learn(&fake::container(
            "CAB-41054b63d372e0dd8117738ffd0d114c",
            vec![named_as(5, a_script("", "SceneHandler"))],
            &[],
        ));

        let other = fake::container(
            "CAB-a6c1d095ac2d442e9eef4bcaf8b9ef14",
            vec![Object::forged(serial::MONO_BEHAVIOUR, 7, a_behaviour(1, 5))],
            &["archive:/CAB-41054b63d372e0dd8117738ffd0d114c/CAB-41054b63d372e0dd8117738ffd0d114c"],
        );

        assert_eq!(
            names.of(&other, &other.objects[0]),
            Some("SceneHandler"),
            "an Addressables bundle is pointed at by its CAB name, never by its file name"
        );
    }

    #[test]
    fn two_containers_handing_out_the_same_path_id_are_told_apart() {
        let mut names = Names::default();
        names.learn(&fake::container(
            "Bundles/one.bundle",
            vec![named_as(5, a_script("", "Alpha"))],
            &[],
        ));
        names.learn(&fake::container(
            "Bundles/two.bundle",
            vec![named_as(5, a_script("", "Beta"))],
            &[],
        ));

        let two = fake::container(
            "Bundles/two.bundle",
            vec![Object::forged(serial::MONO_BEHAVIOUR, 7, a_behaviour(0, 5))],
            &[],
        );

        assert_eq!(
            names.of(&two, &two.objects[0]),
            Some("Beta"),
            "a path id is only unique inside one container, so keying scripts by it alone would \
             name a behaviour after whichever bundle happened to be read last"
        );
    }

    #[test]
    fn a_script_nobody_shipped_leaves_the_behaviour_unnamed() {
        let names = Names::default();
        let alone = fake::container(
            "level0",
            vec![Object::forged(serial::MONO_BEHAVIOUR, 7, a_behaviour(9, 1))],
            &[],
        );

        assert!(
            names.of(&alone, &alone.objects[0]).is_none(),
            "a behaviour pointing at a script this game never shipped cannot be walked, and \
             guessing a class for it would read text out of the wrong bytes"
        );
        assert!(
            names
                .of(
                    &alone,
                    &Object::forged(serial::MONO_BEHAVIOUR, 7, vec![0; 4])
                )
                .is_none(),
            "a body too short to hold a pointer must not panic"
        );
    }
}
