//! Read publisher metadata from a portable payload without launching it.
use quick_xml::{events::Event, Reader};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
};

const MAX_METADATA: u64 = 256 * 1024;

pub(crate) fn portable_version(root: &Path, executable: &Path, app: &str) -> Option<String> {
    if !executable.is_file() {
        return None;
    }
    let ids = crate::catalog::bundle_ids(app);
    let mut image = File::open(executable).ok()?;
    let mut magic = [0; 11];
    image.read_exact(&mut magic).ok()?;
    if &magic[..4] != b"\x7fELF" {
        return None;
    }
    if &magic[8..11] == b"AI\x02" {
        image.rewind().ok()?;
        let stamp = FileStamp::of(&image.metadata().ok()?);
        return image_version(executable, image, stamp, &ids, app);
    }
    for id in ids {
        let path = root.join(format!("share/metainfo/{id}.metainfo.xml"));
        if path.is_file() && crate::files::inside(&path, root).is_ok() {
            if let Ok(file) = File::open(path) {
                if let Some(version) = read_text(file).and_then(|xml| appstream_version(&xml, &id))
                {
                    return Some(version);
                }
            }
        }
    }
    None
}

fn read_text(source: impl Read) -> Option<String> {
    let mut text = String::new();
    source
        .take(MAX_METADATA + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= MAX_METADATA).then_some(text)
}

fn appstream_version(xml: &str, expected_id: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut parents = Vec::new();
    let mut identity = false;
    let mut release = None;
    loop {
        let event = reader.read_event().ok()?;
        let node = match &event {
            Event::Start(node) | Event::Empty(node) => Some(node),
            Event::End(_) => {
                parents.pop();
                None
            }
            Event::DocType(_) => return None,
            Event::Eof => break,
            _ => None,
        };
        if let Some(node) = node {
            let name = node.name();
            if parents.as_slice() == ["component"]
                && name.as_ref() == "id"
                && matches!(event, Event::Start(_))
            {
                identity = reader.read_text(name).ok()?.into_inner().trim() == expected_id;
                continue;
            }
            if parents.as_slice() == ["component", "releases"]
                && name.as_ref() == "release"
                && release.is_none()
            {
                release = Some(node.try_get_attribute("version").ok()??.value.into_owned());
            }
            if matches!(event, Event::Start(_)) {
                parents.push(name.into_inner().to_owned());
            }
        }
    }
    (identity && parents.is_empty())
        .then_some(release)
        .flatten()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp(u64, u64, u64, i64, i64, i64, i64);

impl FileStamp {
    fn of(metadata: &Metadata) -> Self {
        Self(
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    }
}

type ImageVersions = HashMap<(PathBuf, String), (FileStamp, Option<String>)>;

fn image_version(
    path: &Path,
    image: File,
    stamp: FileStamp,
    ids: &[String],
    app: &str,
) -> Option<String> {
    static CACHE: OnceLock<Mutex<ImageVersions>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let key = (path.to_path_buf(), app.to_owned());
    if let Some((_, version)) = cache
        .lock()
        .ok()?
        .get(&key)
        .filter(|(saved, _)| *saved == stamp)
    {
        return version.clone();
    }
    let offset = squashfs_offset(image)?;
    let mut version = None;
    for id in ids {
        let mut child = Command::new("unsquashfs")
            .args(["-cat", "-processors", "1", "-offset"])
            .arg(offset.to_string())
            .arg(path)
            .arg(format!("usr/share/metainfo/{id}.metainfo.xml"))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let xml = child.stdout.take().and_then(read_text);
        if child.wait().ok()?.success() {
            version = xml.and_then(|text| appstream_version(&text, id));
            if version.is_some() {
                break;
            }
        }
    }
    if FileStamp::of(&fs::metadata(path).ok()?) != stamp {
        return None;
    }
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(key, (stamp, version.clone()));
    version
}

fn squashfs_offset(mut image: File) -> Option<u64> {
    let mut header = [0; 64];
    image.read_exact(&mut header).ok()?;
    if header[5] != 1 {
        return None;
    }
    // Type 2 AppImages append SquashFS immediately after the ELF section table.
    let (start, entry, count) = match header[4] {
        1 => (
            u32::from_le_bytes(header[32..36].try_into().ok()?) as u64,
            46,
            48,
        ),
        2 => (u64::from_le_bytes(header[40..48].try_into().ok()?), 58, 60),
        _ => return None,
    };
    let half = |at| u16::from_le_bytes([header[at], header[at + 1]]) as u64;
    let offset = start.checked_add(half(entry) * half(count))?;
    if start < 52 || offset >= image.metadata().ok()?.len() {
        return None;
    }
    image.seek(SeekFrom::Start(offset)).ok()?;
    let mut magic = [0; 4];
    image.read_exact(&mut magic).ok()?;
    (&magic == b"hsqs").then_some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstream_requires_the_selected_apps_component_id() {
        let xml = r#"<component type="desktop-application"><id>ai.storyteller.filmcraft</id>
            <!-- <release version="99.0"/> -->
            <releases><release version="2.0.1"/><release version="2.0.0"/></releases></component>"#;
        assert_eq!(
            appstream_version(xml, "ai.storyteller.filmcraft").as_deref(),
            Some("2.0.1")
        );
        assert_eq!(appstream_version(xml, "ai.storyteller.designcraft"), None);
    }

    #[test]
    #[ignore = "requires squashfs-tools to build and read a real AppImage filesystem"]
    fn replacing_appimage_at_the_same_path_refreshes_its_version() {
        let root = std::env::temp_dir().join(format!("craft-appimage-{}", uuid::Uuid::new_v4()));
        let tree = root.join("tree");
        let xml = tree.join("usr/share/metainfo/ai.storyteller.filmcraft.metainfo.xml");
        fs::create_dir_all(xml.parent().unwrap()).unwrap();
        let exe = root.join("filmcraft");
        let replace = |version| {
            let squashfs = root.join("fs.squashfs");
            fs::write(&xml, format!(
                "<component><id>ai.storyteller.filmcraft</id><releases><release version=\"{version}\"/></releases></component>"
            )).unwrap();
            assert!(Command::new("mksquashfs")
                .arg(&tree)
                .arg(&squashfs)
                .args(["-processors", "1", "-noappend", "-quiet"])
                .status()
                .unwrap()
                .success());
            let mut image = vec![0; 128];
            image[..6].copy_from_slice(b"\x7fELF\x02\x01");
            image[8..11].copy_from_slice(b"AI\x02");
            image[40..48].copy_from_slice(&64_u64.to_le_bytes());
            image[58..60].copy_from_slice(&64_u16.to_le_bytes());
            image[60..62].copy_from_slice(&1_u16.to_le_bytes());
            image.extend(fs::read(squashfs).unwrap());
            let replacement = root.join("replacement");
            fs::write(&replacement, image).unwrap();
            fs::rename(replacement, &exe).unwrap();
        };
        replace("1.0.0");
        let before = portable_version(&root, &exe, "filmcraft");
        replace("1.1.0");
        let after = portable_version(&root, &exe, "filmcraft");
        fs::remove_dir_all(root).unwrap();
        assert_eq!(before.as_deref(), Some("1.0.0"));
        assert_eq!(after.as_deref(), Some("1.1.0"));
    }
}
