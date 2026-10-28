//! Reading files from zip archives — the jars of an Eclipse target platform's bundles — by
//! EMF's archive URIs: `archive:file:/plugins/a_1.0.0.jar!/model/a.ecore` (also `jar:file:…`).
//!
//! Only what jars need: stored and deflated entries, zip64 sizes and offsets. The central
//! directories of opened archives are cached (and read again when the file changes).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// The largest entry that is read (metamodels and manifests are far smaller).
const MAX_ENTRY: u64 = 256 << 20;

/// An opened zip archive: the entries of its central directory.
#[derive(Debug)]
pub struct Archive {
    path: PathBuf,
    entries: HashMap<String, Entry>,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    method: u16,
    compressed: u64,
    size: u64,
    header: u64,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

fn invalid(path: &Path, what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {what}", path.display()))
}

impl Archive {
    /// Opens an archive, reading its central directory.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Archive> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        // The end of central directory record is in the last 64 KiB + 22 bytes.
        let tail_len = len.min(65_557);
        file.seek(SeekFrom::Start(len - tail_len))?;
        let mut tail = vec![0; tail_len as usize];
        file.read_exact(&mut tail)?;
        let eocd = (0..tail.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(&tail, i) == Some(0x0605_4b50))
            .ok_or_else(|| invalid(path, "not a zip archive"))?;
        let mut count = u64::from(u16_at(&tail, eocd + 10).unwrap_or(0));
        let mut size = u64::from(u32_at(&tail, eocd + 12).unwrap_or(0));
        let mut offset = u64::from(u32_at(&tail, eocd + 16).unwrap_or(0));
        // Zip64: the locator precedes the end record and points to the zip64 end record.
        if (count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF)
            && eocd >= 20
            && u32_at(&tail, eocd - 20) == Some(0x0706_4b50)
        {
            let at = u64_at(&tail, eocd - 12).ok_or_else(|| invalid(path, "bad zip64 locator"))?;
            let mut record = [0; 56];
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut record)?;
            if u32_at(&record, 0) != Some(0x0606_4b50) {
                return Err(invalid(path, "bad zip64 end of central directory"));
            }
            count = u64_at(&record, 32).unwrap_or(0);
            size = u64_at(&record, 40).unwrap_or(0);
            offset = u64_at(&record, 48).unwrap_or(0);
        }
        if size > len {
            return Err(invalid(path, "bad central directory"));
        }
        let mut directory = vec![0; size as usize];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut directory)?;
        let mut entries = HashMap::new();
        let mut i = 0;
        for _ in 0..count {
            if u32_at(&directory, i) != Some(0x0201_4b50) {
                return Err(invalid(path, "bad central directory entry"));
            }
            let field = |o: usize| u16_at(&directory, i + o).map(usize::from).unwrap_or(0);
            let (name_len, extra_len, comment_len) = (field(28), field(30), field(32));
            let name = directory.get(i + 46..i + 46 + name_len).ok_or_else(|| invalid(path, "truncated entry"))?;
            let name = String::from_utf8_lossy(name).into_owned();
            let mut entry = Entry {
                method: u16_at(&directory, i + 10).unwrap_or(0),
                compressed: u64::from(u32_at(&directory, i + 20).unwrap_or(0)),
                size: u64::from(u32_at(&directory, i + 24).unwrap_or(0)),
                header: u64::from(u32_at(&directory, i + 42).unwrap_or(0)),
            };
            // Zip64 values, in the order of the fields that overflowed.
            let extra = directory.get(i + 46 + name_len..i + 46 + name_len + extra_len).unwrap_or(&[]);
            let mut j = 0;
            while j + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, j).unwrap_or(0), usize::from(u16_at(extra, j + 2).unwrap_or(0)));
                if id == 1 {
                    let mut k = j + 4;
                    for value in [&mut entry.size, &mut entry.compressed, &mut entry.header] {
                        if *value == 0xFFFF_FFFF {
                            *value = u64_at(extra, k).unwrap_or(*value);
                            k += 8;
                        }
                    }
                }
                j += 4 + len;
            }
            entries.insert(name, entry);
            i += 46 + name_len + extra_len + comment_len;
        }
        Ok(Archive { path: path.to_path_buf(), entries })
    }

    /// Whether the archive has an entry (a path with `/`s, without a leading one).
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// The contents of an entry.
    pub fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let entry = *self.entries.get(name).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}!/{name}: no such entry", self.path.display()))
        })?;
        if entry.size > MAX_ENTRY || entry.compressed > MAX_ENTRY {
            return Err(invalid(&self.path, "entry too large"));
        }
        let mut file = File::open(&self.path)?;
        let mut header = [0; 30];
        file.seek(SeekFrom::Start(entry.header))?;
        file.read_exact(&mut header)?;
        if u32_at(&header, 0) != Some(0x0403_4b50) {
            return Err(invalid(&self.path, "bad local header"));
        }
        let skip = u64::from(u16_at(&header, 26).unwrap_or(0)) + u64::from(u16_at(&header, 28).unwrap_or(0));
        file.seek(SeekFrom::Current(skip as i64))?;
        let mut data = vec![0; entry.compressed as usize];
        file.read_exact(&mut data)?;
        match entry.method {
            0 => Ok(data),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&data, entry.size.max(1) as usize)
                .map_err(|e| invalid(&self.path, &format!("{name}: cannot inflate ({e:?})"))),
            m => Err(invalid(&self.path, &format!("{name}: unsupported compression method {m}"))),
        }
    }

    /// The contents of an entry as text (UTF-8, a BOM removed).
    pub fn read_to_string(&self, name: &str) -> std::io::Result<String> {
        let bytes = self.read(name)?;
        let text = String::from_utf8(bytes).map_err(|_| invalid(&self.path, &format!("{name}: not UTF-8")))?;
        Ok(text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text))
    }
}

type Cache = Mutex<HashMap<PathBuf, (Option<SystemTime>, u64, Arc<Archive>)>>;

/// An archive, from the cache while the file is unchanged.
pub fn open_cached(path: &Path) -> std::io::Result<Arc<Archive>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let metadata = std::fs::metadata(path)?;
    let stamp = (metadata.modified().ok(), metadata.len());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((modified, len, archive)) = cache.lock().ok().and_then(|c| c.get(path).cloned())
        && (modified, len) == stamp
    {
        return Ok(archive);
    }
    let archive = Arc::new(Archive::open(path)?);
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_path_buf(), (stamp.0, stamp.1, archive.clone()));
    }
    Ok(archive)
}

/// The archive and the entry an archive URI (`archive:file:/a.jar!/model/a.ecore`,
/// `jar:file:…`) stands for.
pub fn split_uri(uri: &str) -> Option<(PathBuf, &str)> {
    let rest = uri.strip_prefix("archive:").or_else(|| uri.strip_prefix("jar:"))?;
    let (archive, entry) = rest.split_once("!/")?;
    Some((file_uri_path(archive)?, entry))
}

/// The archive URI of an entry of a jar.
pub fn uri(archive: &Path, entry: &str) -> String {
    format!("archive:{}!/{entry}", file_uri(archive))
}

/// A `file:` URI for a path (`file:/a/b`, `file:/C:/a/b`), with `%`, `#`, `?` and spaces escaped.
pub fn file_uri(path: &Path) -> String {
    let mut p = path.to_string_lossy().replace('\\', "/");
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    let mut out = String::from("file:");
    for c in p.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            ' ' => out.push_str("%20"),
            c => out.push(c),
        }
    }
    out
}

/// The path of a `file:` URI.
pub fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let mut bytes = Vec::new();
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && let Some(b) = rest.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            bytes.push(b);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let path = String::from_utf8(bytes).ok()?;
    // `/C:/a` on Windows.
    let path = match path.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() && cfg!(windows) => path[1..].to_string(),
        _ => path,
    };
    Some(PathBuf::from(path))
}

/// Reads the text of an archive URI.
pub fn read_uri(uri: &str) -> Option<String> {
    let (path, entry) = split_uri(uri)?;
    open_cached(&path).ok()?.read_to_string(entry).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip() {
        let p = Path::new("/a b/c#d%e.jar");
        assert_eq!(file_uri(p), "file:/a%20b/c%23d%25e.jar");
        assert_eq!(file_uri_path(&file_uri(p)).unwrap(), p);
        assert_eq!(file_uri_path("file:///x/y").unwrap(), Path::new("/x/y"));
        let (jar, entry) = split_uri("archive:file:/p/a_1.0.jar!/model/a.ecore").unwrap();
        assert_eq!((jar.as_path(), entry), (Path::new("/p/a_1.0.jar"), "model/a.ecore"));
    }
}
