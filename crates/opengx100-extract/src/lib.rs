//! Gets the pedal's parameter tables from the user's own BOSS TONE STUDIO, so that nothing of Roland's
//! ships with the editor. At first launch the official editor's configuration scripts are run once in
//! an embedded JavaScript engine (boa) by `tools/extract/core.js`, the same code the development tool
//! (`tools/extract/extract.mjs`) runs under Node; the result is cached and re-extracted when BOSS TONE
//! STUDIO changes version. The tables say which pedal they are for (GX-100, GX-10, …).

use boa_engine::{js_string, Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Source};
use opengx100_protocol::tables::{self, Tables};
use std::fmt;
use std::path::{Path, PathBuf};

/// Where the official installer puts it on macOS.
pub const DEFAULT_APP: &str = "/Applications/BOSS/GX-100/BOSS TONE STUDIO for GX-100.app";
#[cfg(not(windows))]
const APP_NAME: &str = "BOSS TONE STUDIO for GX-100.app";

/// The extraction script, shared with the Node tool.
const CORE_JS: &str = include_str!("../../../tools/extract/core.js");

/// The official editor's files `core.js` reads, relative to the html folder. The `.elf` (the UI definition:
/// bg777-pc.elf on the GX-100, bg846-pc.elf on the GX-10) is added by [`elf_name`].
const FILES: &[&str] = &[
    "js/utilities/constant.js",
    "js/utilities/converter.js",
    "js/config/product_setting.js",
    "js/config/address_map.js",
    "js/businesslogic/address_const.js",
    "js/config/resource.js",
    "js/businesslogic/chain/chain_controller.js",
    "js/config/effect_parameter.js",
    "js/businesslogic/numpad_const.js",
    "js/businesslogic/assign_const.js",
    "js/config/librarian_setting.js",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// BOSS TONE STUDIO isn't installed where we looked (and nothing is cached).
    AppNotFound,
    /// The path given isn't a BOSS TONE STUDIO with the files we read.
    NotTheApp(PathBuf),
    /// Reading its files or running its scripts failed (e.g. an update changed them).
    Extract(String),
    /// The tables loaded but are unusable, or the cache couldn't be written.
    Tables(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::AppNotFound => write!(f, "BOSS TONE STUDIO isn't installed (looked in /Applications)"),
            Error::NotTheApp(p) => write!(f, "{} isn't BOSS TONE STUDIO", p.display()),
            Error::Extract(e) => write!(f, "reading BOSS TONE STUDIO's tables failed: {e}"),
            Error::Tables(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The html folder: inside the app bundle on macOS, the folder itself when it was unpacked, or a folder that
/// holds it (Windows keeps it as a zip inside the .exe; see [`unpack`]).
fn html(app: &Path) -> PathBuf {
    let mac = app.join("Contents/Resources/html");
    if mac.is_dir() {
        mac
    } else if app.join("js/config/product_setting.js").is_file() {
        app.to_path_buf()
    } else {
        app.join("html")
    }
}

/// The editor's UI definition: `bg<model>-pc.elf` (GX-100: bg777, GX-10: bg846). An install can carry more than
/// one — the GX-100's has bg709 and bg777, and bg709 comes first — so the model in `product_setting.js` picks, and
/// only a lone file is taken as it is.
fn elf_name(dir: &Path) -> Option<String> {
    let product = std::fs::read_to_string(dir.join("js/config/product_setting.js")).ok()?;
    let known = match product_name(&product)?.as_str() {
        "GX-100" => "bg777-pc.elf",
        "GX-10" => "bg846-pc.elf",
        _ => "",
    };
    if dir.join(known).is_file() {
        return Some(known.to_string());
    }
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("bg") && n.ends_with("-pc.elf"))
        .collect();
    // No single candidate: better no labels at all than another model's.
    match found.len() {
        1 => found.pop(),
        _ => None,
    }
}

/// The product's name from `product_setting.js` (`name:\t'GX-10',`).
fn product_name(js: &str) -> Option<String> {
    let line = js.lines().map(str::trim_start).find(|l| l.starts_with("name:"))?;
    let value = line["name:".len()..].trim_start().trim_start_matches(['\'', '"']);
    let end = value.find(['\'', '"', ',', '\n', ' ', '\t'])?;
    Some(value[..end].to_string())
}

/// Whether `app` looks like BOSS TONE STUDIO we can read (has the files we read).
pub fn is_app(app: &Path) -> bool {
    let dir = html(app);
    elf_name(&dir).is_some() && FILES.iter().all(|f| dir.join(f).is_file())
}

/// The installed BOSS TONE STUDIO as a folder that holds `html/`: the macOS app bundle, or (Windows) its html
/// tree unpacked out of the .exe, which keeps it as a zip.
#[cfg(windows)]
pub fn find_app() -> Option<PathBuf> {
    windows_apps().find_map(|exe| unpack(&exe).ok())
}

/// BOSS TONE STUDIO's .exe files: `<Program Files>\BOSS\BOSS TONE STUDIO for <pedal>\*.exe`.
#[cfg(windows)]
fn windows_apps() -> impl Iterator<Item = PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|p| p.join("BOSS"))
        .filter(|p| p.is_dir())
        .flat_map(|p| std::fs::read_dir(p).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .flat_map(|p| std::fs::read_dir(p).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
}

/// The spread of one zip inside another file: (start, end) of the archive. The .exe carries its own data after
/// the archive, so the end-of-central-directory record is not at the end: locate it, then work out where the
/// archive starts.
#[cfg(any(windows, test))]
fn carve(bytes: &[u8]) -> Option<(usize, usize)> {
    const EOCD: &[u8] = b"PK\x05\x06";
    let eocd = bytes.windows(EOCD.len()).rposition(|w| w == EOCD)?;
    let u32_at =
        |o: usize| -> Option<usize> { Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as usize) };
    let cd_size = u32_at(eocd + 12)?;
    let cd_off = u32_at(eocd + 16)?;
    let comment_len = u16::from_le_bytes(bytes.get(eocd + 20..eocd + 22)?.try_into().ok()?) as usize;
    let start = eocd.checked_sub(cd_size + cd_off)?;
    let end = (eocd + 22 + comment_len).min(bytes.len());
    (start < end).then_some((start, end))
}

/// Unpacks the html tree out of BOSS TONE STUDIO's .exe into `<data dir>/bts/<name>-<size>-<mtime>/` (once),
/// and returns that folder.
#[cfg(windows)]
fn unpack(exe: &Path) -> Result<PathBuf, Error> {
    let bytes = std::fs::read(exe).map_err(|e| Error::Extract(format!("{}: {e}", exe.display())))?;
    let (start, end) = carve(&bytes).ok_or_else(|| Error::NotTheApp(exe.to_path_buf()))?;
    let meta = std::fs::metadata(exe).map_err(|e| Error::Extract(format!("{}: {e}", exe.display())))?;
    let stamp =
        meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let name = exe.file_stem().map_or_else(|| "bts".into(), |n| n.to_string_lossy().into_owned());
    let dir = data_dir()
        .ok_or_else(|| Error::Extract("no data folder on this system".into()))?
        .join("bts")
        .join(format!("{name}-{}-{stamp}", meta.len()));
    if dir.join("html/js/config/product_setting.js").is_file() {
        return Ok(dir);
    }
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(&bytes[start..end]))
        .map_err(|e| Error::Extract(format!("{}: {e}", exe.display())))?;
    for i in 0..ar.len() {
        let mut entry = ar.by_index(i).map_err(|e| Error::Extract(format!("{}: {e}", exe.display())))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if rel.components().next().is_none_or(|c| c.as_os_str() != "html") {
            continue;
        }
        let out = dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| Error::Extract(format!("{}: {e}", out.display())))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Extract(format!("{}: {e}", out.display())))?;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| Error::Extract(format!("{}: {e}", out.display())))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| Error::Extract(format!("{}: {e}", out.display())))?;
    }
    Ok(dir)
}

/// The installed BOSS TONE STUDIO for GX-100: the default location, else anywhere a couple of levels
/// below /Applications or ~/Applications.
#[cfg(not(windows))]
pub fn find_app() -> Option<PathBuf> {
    let default = PathBuf::from(DEFAULT_APP);
    if is_app(&default) {
        return Some(default);
    }
    let mut dirs: Vec<PathBuf> = vec!["/Applications".into()];
    dirs.extend(dirs::home_dir().map(|h| h.join("Applications")));
    for depth in 0..3 {
        let mut next = Vec::new();
        for d in &dirs {
            let Ok(entries) = std::fs::read_dir(d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.file_name().is_some_and(|n| n == APP_NAME) && is_app(&p) {
                    return Some(p);
                }
                // Don't descend into other apps' bundles.
                if depth < 2 && p.is_dir() && p.extension().is_none_or(|x| x != "app") {
                    next.push(p);
                }
            }
        }
        dirs = next;
    }
    None
}

/// The folder to read from: `path` itself when it holds the editor's files, else (Windows) the html tree
/// unpacked out of BOSS TONE STUDIO's .exe.
pub fn resolve(path: &Path) -> Result<PathBuf, Error> {
    if is_app(path) {
        return Ok(path.to_path_buf());
    }
    #[cfg(windows)]
    if path.is_file() {
        return unpack(path);
    }
    Err(Error::NotTheApp(path.to_path_buf()))
}

/// `key: 'value'` from the official editor's product settings (without running them).
fn pick(src: &str, key: &str) -> Option<String> {
    let mut rest = src;
    while let Some(i) = rest.find(key) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + key.len()..].trim_start();
        rest = &rest[i + key.len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(v) = after.strip_prefix(':') else { continue };
        let v = v.trim_start().trim_start_matches('\'');
        let end = v.find(|c: char| c == '\'' || c == ',' || c.is_whitespace()).unwrap_or(v.len());
        if end > 0 {
            return Some(v[..end].to_string());
        }
    }
    None
}

/// BOSS TONE STUDIO's version and build ("2.0.1", "118"), read from its product settings.
pub fn app_version(app: &Path) -> Option<(String, String)> {
    let src = std::fs::read_to_string(html(app).join("js/config/product_setting.js")).ok()?;
    Some((pick(&src, "version")?, pick(&src, "build")?))
}

fn js_err(msg: String) -> boa_engine::JsError {
    JsNativeError::error().with_message(msg).into()
}

/// Runs the official editor's configuration scripts and returns the tables as JSON (the format
/// [`tables::from_json`] reads). Takes a few seconds.
pub fn extract(app: &Path) -> Result<String, Error> {
    let app = resolve(app)?;
    let dir = html(&app);
    let elf = elf_name(&dir).ok_or_else(|| Error::NotTheApp(app.to_path_buf()))?;
    let mut ctx = Context::default();
    let fail = |e: boa_engine::JsError| Error::Extract(e.to_string());

    // The files, as one object of strings: core.js asks for them by name.
    let files = JsObject::with_null_proto();
    for f in FILES.iter().copied().chain(std::iter::once(elf.as_str())) {
        let text = std::fs::read_to_string(dir.join(f)).map_err(|e| Error::Extract(format!("{f}: {e}")))?;
        files.set(JsString::from(f), JsString::from(text.as_str()), false, &mut ctx).map_err(fail)?;
    }
    // The chain block images, for their colours (optional: without them, blocks keep the default look).
    if let Ok(images) = std::fs::read_dir(dir.join("blockImages")) {
        for e in images.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("EFFECTS_") && name.ends_with(".svg")) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                let key = JsString::from(format!("blockImages/{name}").as_str());
                files.set(key, JsString::from(text.as_str()), false, &mut ctx).map_err(fail)?;
            }
        }
    }
    ctx.register_global_property(js_string!("__files"), files, boa_engine::property::Attribute::all()).map_err(fail)?;
    // Runs one of the official editor's scripts in the shared global scope.
    let run = NativeFunction::from_copy_closure(|_, args, ctx| -> JsResult<JsValue> {
        let src = args.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
        ctx.eval(Source::from_bytes(src.as_bytes()))
    });
    ctx.register_global_builtin_callable(js_string!("__run"), 2, run).map_err(fail)?;

    let script = format!(
        "{CORE_JS}\n;(() => {{\n\
           globalThis.console = {{ log() {{}}, warn() {{}}, error() {{}}, info() {{}}, debug() {{}} }};\n\
           const read = (rel) => {{\n\
             if (!(rel in __files)) throw new Error('core.js asked for an unlisted file: ' + rel);\n\
             return __files[rel];\n\
           }};\n\
           const t = extractTables({{ read, run: __run, global: globalThis, elf: {elf:?}, log() {{}} }});\n\
           return JSON.stringify(tablesBundle(t));\n\
         }})()"
    );
    let out = ctx.eval(Source::from_bytes(script.as_bytes())).map_err(fail)?;
    let json = out.as_string().ok_or_else(|| js_err("no result".into())).map_err(fail)?;
    Ok(json.to_std_string_escaped())
}

/// The editor's data folder: `<data dir>/OpenGX100 Editor` (on macOS `~/Library/Application Support/OpenGX100 Editor`).
/// Data from before the project was renamed (`gx100-editor`) is moved there the first time.
pub fn data_dir() -> Option<PathBuf> {
    let base = dirs::data_dir()?;
    let dir = base.join("OpenGX100 Editor");
    let old = base.join("gx100-editor");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    Some(dir)
}

/// Where extracted tables are kept: `<data dir>/tables/` (on macOS
/// `~/Library/Application Support/OpenGX100 Editor/tables/`). A `docs.json` (parameter guide text, from
/// `tools/extract/manual.mjs`) placed there is loaded too.
pub fn cache_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tables"))
}

fn cache_file() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("tables.json"))
}

/// Where the tables came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `$OPENGX100_TABLES`.
    Env(PathBuf),
    /// The cache, still matching the installed BOSS TONE STUDIO (or it's no longer installed).
    Cache(PathBuf),
    /// Freshly extracted from this app (and cached).
    Extracted(PathBuf),
}

/// The tables to use: `$OPENGX100_TABLES` if set; else the cache, if it matches the installed BOSS TONE
/// STUDIO (`app`, or [`find_app`]); else extracted from it now and cached. Doesn't install them.
pub fn prepare(app: Option<&Path>) -> Result<(Tables, Origin), Error> {
    if let Some(p) = std::env::var_os("OPENGX100_TABLES").map(PathBuf::from) {
        return tables::load_file(&p).map(|t| (t, Origin::Env(p))).map_err(Error::Tables);
    }
    let app = app.map(Path::to_path_buf).or_else(find_app).map(|p| resolve(&p)).transpose()?;
    let cached = cache_file().filter(|f| f.is_file()).and_then(|f| tables::load_file(&f).ok().map(|t| (t, f)));
    match (app, cached) {
        (None, Some((t, f))) => Ok((t, Origin::Cache(f))),
        (None, None) => Err(Error::AppNotFound),
        (Some(app), cached) => {
            let version = app_version(&app);
            if let Some((t, f)) = cached {
                let same =
                    version.as_ref().is_some_and(|(v, b)| t.product.bts_version == v && t.product.bts_build == b);
                if same {
                    return Ok((t, Origin::Cache(f)));
                }
            }
            let json = extract(&app)?;
            let t = tables::from_json(&json, None).map_err(Error::Tables)?;
            let docs = cache_dir().map(|d| d.join("docs.json")).and_then(|d| std::fs::read_to_string(d).ok());
            let t = match docs {
                Some(d) => tables::from_json(&json, Some(&d)).unwrap_or(t),
                None => t,
            };
            if let Some(f) = cache_file() {
                write_atomic(&f, &json).map_err(|e| Error::Tables(format!("saving {}: {e}", f.display())))?;
            }
            Ok((t, Origin::Extracted(app)))
        }
    }
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_product_settings() {
        let src = "var ProductSetting = {\n  name: 'GX-100',\n  version: '2.0.1', build: '118',\n  subversion: 'x' };";
        assert_eq!(pick(src, "version").as_deref(), Some("2.0.1"));
        assert_eq!(pick(src, "build").as_deref(), Some("118"));
        assert_eq!(pick(src, "name").as_deref(), Some("GX-100"));
        assert_eq!(pick(src, "missing"), None);
    }

    /// The GX-100's install carries both bg709-pc.elf and bg777-pc.elf, and bg709 comes first in the folder
    /// listing; the product name has to pick, not the order.
    #[test]
    fn picks_the_ui_definition_by_product() {
        let base = std::env::temp_dir().join(format!("opengx100-elf-{}", std::process::id()));
        let make = |name: &str, product: &str, elfs: &[&str]| {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("js/config")).unwrap();
            std::fs::write(
                dir.join("js/config/product_setting.js"),
                format!("var P = {{\n\tname:\t'{product}',\n\tversion:\t'1.0.0',\n}};"),
            )
            .unwrap();
            for elf in elfs {
                std::fs::write(dir.join(elf), "{}").unwrap();
            }
            dir
        };

        let gx100 = make("gx100", "GX-100", &["bg709-pc.elf", "bg777-pc.elf", "other.elf"]);
        assert_eq!(elf_name(&gx100).as_deref(), Some("bg777-pc.elf"));

        let gx10 = make("gx10", "GX-10", &["bg846-pc.elf"]);
        assert_eq!(elf_name(&gx10).as_deref(), Some("bg846-pc.elf"));

        // No known name and more than one candidate: no labels beats another model's.
        let odd = make("odd", "GX-99", &["bg709-pc.elf", "bg777-pc.elf"]);
        assert_eq!(elf_name(&odd), None);

        // A lone file is still taken as it is.
        let lone = make("lone", "GX-99", &["bg123-pc.elf"]);
        assert_eq!(elf_name(&lone).as_deref(), Some("bg123-pc.elf"));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Windows keeps the html tree as a zip inside the .exe, with the exe's own data after the archive.
    #[test]
    fn carves_a_zip_out_of_an_exe() {
        use std::io::{Read, Write};
        let mut buf = b"NONSENSE EXE HEADER".to_vec();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file("html/js/config/product_setting.js", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"var x = 1;").unwrap();
            w.finish().unwrap();
        }
        let archive_end = buf.len();
        buf.extend_from_slice(&[0u8; 4096]);
        let (start, end) = carve(&buf).expect("archive found");
        assert_eq!(end, archive_end);
        let mut ar = zip::ZipArchive::new(std::io::Cursor::new(&buf[start..end])).unwrap();
        let mut s = String::new();
        ar.by_name("html/js/config/product_setting.js").unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "var x = 1;");
    }

    /// Runs the real extraction when BOSS TONE STUDIO is installed and compares it with the Node
    /// tool's output (`node tools/extract/extract.mjs` → extracted/tables.json), when present.
    #[test]
    #[ignore = "needs BOSS TONE STUDIO installed; slow"]
    fn matches_the_node_tool() {
        let app = find_app().expect("BOSS TONE STUDIO installed");
        let t0 = std::time::Instant::now();
        let json = extract(&app).expect("extract");
        eprintln!("extracted in {:?}", t0.elapsed());
        let t = tables::from_json(&json, None).expect("tables");
        assert!(t.params.len() > 2000 && t.fx_types.len() > 50);
        let node = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extracted/tables.json");
        if let Ok(n) = std::fs::read_to_string(node) {
            let parse = |s: &str| opengx100_protocol::json::Json::parse(s).unwrap();
            assert!(parse(&json) == parse(&n), "boa and Node disagree");
            eprintln!("identical to the Node tool's output");
        }
    }
}
