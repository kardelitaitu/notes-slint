//! Byte-exact round-trip fixtures: a deterministic generator and a verifier.
//!
//! docs/features.md 4.5 and the "do no harm" rule: loading then saving a
//! foreign file must be byte-identical — encoding, BOM, line endings and
//! trailing newline survive. These fixtures are the gate for that rule, and
//! per decision D8 they are BYTE-AUTHORED: no editor ever touches them,
//! because an editor tidies a missing trailing newline away and the file then
//! proves nothing. The generator writes every byte; the verifier proves the
//! tree still matches.
//!
//! Content derivation (reviewable by reading this file):
//! * One documented base text, below — three lines, LF-terminated, containing
//!   one non-ASCII character (é) so the ansi1252 rows are genuinely ANSI
//!   (a pure-ASCII CP1252 file is indistinguishable from UTF-8). Every
//!   character costs one or two UTF-8 bytes and exactly two UTF-16 bytes, so
//!   the UTF-16 fixtures are two bytes per character plus their 2-byte BOM
//!   (56 characters -> 114 bytes), the UTF-8 files are 57 bytes (é costs a
//!   second UTF-8 byte), and the ansi1252 files are 56 bytes.
//! * The fixed product crosses encoding x line-ending x trailing-newline:
//!   5 encodings x 2 EOLs x 2 trailings = 20 files. The BOM field of the D8
//!   name is determined by the encoding (utf8bom/utf16le/utf16be always carry
//!   a BOM; utf8/ansi1252 never do), so there are no impossible combinations
//!   to skip: a BOM-less utf16le is not part of this product.
//! * Hand-picked edge cases (empty, lone CR, BOM-only, mixed EOLs, a
//!   character outside CP1252, the documented frontmatter block, and two
//!   foreign .json files) are listed verbatim below.
//! * manifest.json states the count, the ansi_codepage a core test must pass
//!   to detect() for the ansi1252 rows to detect as ansi1252, and one entry
//!   per file with the SAME vocabulary as core::encoding's Detected /
//!   api::FileMeta: encodings utf8, utf8bom, utf16le, utf16be, ansi1252; line
//!   endings lf, crlf (the dominant rule in core::encoding: lone CR and
//!   LF/CRLF mixes report lf). Hashes are computed by this file's own
//!   SHA-256, never taken from Git.
//!
//! The verifier is deliberately self-referential, because a gate that only
//! checks disk-against-manifest dies with its own input file: verify rebuilds
//! the expected manifest in memory from the same spec tables generate uses,
//! diffs the on-disk manifest against it (rows, count, every metadata field),
//! checks every file's bytes against the expected hash, and — always, not
//! behind a flag — regenerates into a tempdir and byte-compares the whole
//! tree. Hand-editing a fixture AND recomputing its hash still fails.
//!
//! Writes are atomic per file: the payload goes to `<name>.new` in the same
//! directory, is flushed and fsynced, and the finished side file is renamed
//! over `<name>` - a concurrent reader sees the previous bytes or the complete
//! new ones, never a mixture (a truncate-in-place write was proven to tear:
//! 12 partial manifest reads and 432 silent fixture reads in 3000 rounds).
//! The manifest is written LAST by the same rule, so a complete manifest
//! implies complete fixtures - the reason the verifier can trust a manifest
//! it can parse.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FIXTURES_REL: &str = "crates/core/tests/fixtures";
const MANIFEST_NAME: &str = "manifest.json";

/// The code page a detect() call must be given for the ansi1252 rows to
/// detect as ansi1252 (with None they report utf8 — see detect_encoding).
/// core::roundtrip passes Some(1252); the eventual bridge passes GetACP().
/// Recorded here so a test can assert the two agree.
const ANSI_CODEPAGE: u64 = 1252;

/// The one documented base text: pure lines, LF-terminated; see module docs.
const BASE_TEXT: &str = "alpha bravo charlie\ndelta écho foxtrot\ngolf hotel india\n";

/// The base text serialized as JSON — same three lines, so the foreign
/// .json fixtures are derived from the same source as everything else.
const BASE_JSON: &str = "{\n  \"lines\": [\n    \"alpha bravo charlie\",\n    \"delta écho foxtrot\",\n    \"golf hotel india\"\n  ]\n}\n";

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

/// One fixture file: its bytes and the manifest metadata for it.
struct Spec {
    file: String,
    bytes: Vec<u8>,
    encoding: &'static str,
    line_ending: &'static str,
    trailing_newline: bool,
    bom_present: bool,
    note: String,
}

/// The five encodings of the product and how to produce their bytes.
#[derive(Clone, Copy)]
enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi1252,
}

const ALL_ENCODINGS: [Enc; 5] = [
    Enc::Utf8,
    Enc::Utf8Bom,
    Enc::Utf16Le,
    Enc::Utf16Be,
    Enc::Ansi1252,
];

impl Enc {
    fn name(self) -> &'static str {
        match self {
            Enc::Utf8 => "utf8",
            Enc::Utf8Bom => "utf8bom",
            Enc::Utf16Le => "utf16le",
            Enc::Utf16Be => "utf16be",
            Enc::Ansi1252 => "ansi1252",
        }
    }

    fn bom(self) -> Option<&'static [u8]> {
        match self {
            Enc::Utf8 | Enc::Ansi1252 => None,
            Enc::Utf8Bom => Some(UTF8_BOM),
            Enc::Utf16Le => Some(UTF16LE_BOM),
            Enc::Utf16Be => Some(UTF16BE_BOM),
        }
    }

    /// The BOM field of the D8 name is decided by the encoding, which is why
    /// the impossible combinations (utf8bom without BOM, BOM-less UTF-16,
    /// ANSI with a BOM) never enter the product.
    fn bom_field(self) -> &'static str {
        if self.bom().is_some() { "bom" } else { "nobom" }
    }

    fn encode(self, text: &str) -> Result<Vec<u8>, String> {
        let body = match self {
            Enc::Utf8 | Enc::Utf8Bom => text.as_bytes().to_vec(),
            Enc::Utf16Le => utf16_bytes(text, false),
            Enc::Utf16Be => utf16_bytes(text, true),
            Enc::Ansi1252 => to_cp1252(text)?,
        };
        match self.bom() {
            Some(bom) => {
                let mut bytes = bom.to_vec();
                bytes.extend_from_slice(&body);
                Ok(bytes)
            }
            None => Ok(body),
        }
    }
}

fn utf16_bytes(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        if big_endian {
            bytes.extend_from_slice(&unit.to_be_bytes());
        } else {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes
}

/// CP1252 for the generator's purposes: the base text only ever contains
/// characters at or below U+00FF, which map one-to-one. Anything above that
/// is refused loudly — the decoder's full CP1252 table lives in core.
fn to_cp1252(text: &str) -> Result<Vec<u8>, String> {
    text.chars()
        .map(|c| {
            u8::try_from(u32::from(c)).map_err(|_| format!("character {c:?} is outside CP1252"))
        })
        .collect()
}

/// Strip the WHOLE final line terminator — \r\n for CRLF text, \n for LF
/// text. One function for every content source: "nonl" means NO terminator
/// bytes remain, not merely "the last byte is not 0x0A" (a lone CR left
/// behind is exactly the harm this gate exists to catch).
fn strip_final_terminator(mut text: String, crlf: bool) -> String {
    if crlf {
        if text.ends_with("\r\n") {
            text.truncate(text.len() - 2);
        }
    } else if text.ends_with('\n') {
        text.pop();
    }
    text
}

/// Apply the EOL rule to a source text, then the trailing rule.
fn with_eol_and_trailing(source: &str, crlf: bool, keep_nl: bool) -> String {
    let text = if crlf {
        source.replace('\n', "\r\n")
    } else {
        source.to_string()
    };
    if keep_nl {
        text
    } else {
        strip_final_terminator(text, crlf)
    }
}

fn product_text(crlf: bool, keep_nl: bool) -> String {
    with_eol_and_trailing(BASE_TEXT, crlf, keep_nl)
}

/// The base JSON under the EOL/trailing rules, for the foreign .json rows.
fn product_json_text(crlf: bool, keep_nl: bool) -> String {
    with_eol_and_trailing(BASE_JSON, crlf, keep_nl)
}

fn product_specs() -> Result<Vec<Spec>, String> {
    let mut specs = Vec::new();
    for enc in ALL_ENCODINGS {
        for (crlf, eol_name) in [(false, "lf"), (true, "crlf")] {
            for (keep_nl, nl_name) in [(true, "nl"), (false, "nonl")] {
                let text = product_text(crlf, keep_nl);
                specs.push(Spec {
                    file: format!(
                        "{}__{}__{}__{}.txt",
                        enc.name(),
                        eol_name,
                        enc.bom_field(),
                        nl_name
                    ),
                    bytes: enc.encode(&text)?,
                    encoding: enc.name(),
                    line_ending: eol_name,
                    trailing_newline: keep_nl,
                    bom_present: enc.bom().is_some(),
                    note: format!(
                        "documented base text; {} line endings; {}; trailing {}",
                        eol_name,
                        enc.bom_field(),
                        nl_name
                    ),
                });
            }
        }
    }
    Ok(specs)
}

/// The hand-picked edge cases. Each content is written out here verbatim so a
/// reviewer can read every byte of the set without running anything.
fn edge_specs() -> Vec<Spec> {
    vec![
        Spec {
            file: "edge__empty.notes".to_string(),
            bytes: Vec::new(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: false,
            bom_present: false,
            note: "empty file, zero bytes: writing a BOM or a newline into someone's empty file is a harm".to_string(),
        },
        Spec {
            file: "edge__lone-cr.notes".to_string(),
            bytes: b"alpha\rbeta\rgamma".to_vec(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: false,
            bom_present: false,
            note: "classic Mac lone-CR line endings; core::encoding reports the default lf, and the round trip must preserve every 0x0D byte".to_string(),
        },
        Spec {
            file: "edge__bom-only.notes".to_string(),
            bytes: UTF8_BOM.to_vec(),
            encoding: "utf8bom",
            line_ending: "lf",
            trailing_newline: false,
            bom_present: true,
            note: "a file that is only a BOM".to_string(),
        },
        Spec {
            file: "edge__mixed-eol.notes".to_string(),
            bytes: b"alpha\nbeta\r\ngamma\r".to_vec(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: false,
            bom_present: false,
            note: "mixed LF then CRLF then lone CR; core::encoding reports the dominant lf (a lone LF beats CRLF)".to_string(),
        },
        Spec {
            file: "edge__outside-ansi1252.txt".to_string(),
            bytes: "café — 😀\n".as_bytes().to_vec(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: true,
            bom_present: false,
            note: "UTF-8 containing U+1F600, outside CP1252: the ansi1252 tests point at this file".to_string(),
        },
        Spec {
            file: "edge__frontmatter.notes".to_string(),
            bytes: concat!(
                "---\n",
                "created: 2026-09-10T08:58:00+08:00\n",
                "pinned: true\n",
                "---\n",
                "\n",
                "alpha bravo charlie\n",
                "delta écho foxtrot\n",
                "golf hotel india\n",
            )
            .as_bytes()
            .to_vec(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: true,
            bom_present: false,
            note: "frontmatter block exactly as docs/features.md shows, body from the documented base text".to_string(),
        },
        Spec {
            file: "utf8__lf__nobom__nl.json".to_string(),
            bytes: BASE_JSON.as_bytes().to_vec(),
            encoding: "utf8",
            line_ending: "lf",
            trailing_newline: true,
            bom_present: false,
            note: "foreign extension: ADR-0001 arms autosave only after an explicit save, so a file the app did not create must not be rewritten".to_string(),
        },
        Spec {
            file: "utf16le__crlf__bom__nonl.json".to_string(),
            bytes: Enc::Utf16Le.encode(&product_json_text(true, false)).expect("base json needs no CP1252"),
            encoding: "utf16le",
            line_ending: "crlf",
            trailing_newline: false,
            bom_present: true,
            note: "foreign extension, UTF-16LE: the same no-rewrite rule for a file core detects as UTF-16".to_string(),
        },
    ]
}

/// Every fixture, sorted by file name: the deterministic order used for both
/// writing and the manifest.
fn all_specs() -> Result<Vec<Spec>, String> {
    let mut specs = product_specs()?;
    specs.extend(edge_specs());
    specs.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(specs)
}

fn extension_of(file: &str) -> &str {
    file.rsplit('.').next().unwrap_or("")
}

/// One manifest row. Keys are emitted in this literal's INSERTION order:
/// serde_json's preserve_order feature is in the dependency graph (via
/// gpui_util), so insertion order is what you get. The literal is kept
/// alphabetical on purpose, and
/// manifest_keys_are_alphabetical_regardless_of_feature_flags enforces it —
/// insert new keys in alphabetical position, or -p xtask and --workspace
/// builds will disagree about the manifest bytes.
fn manifest_row(spec: &Spec) -> Value {
    let extension = extension_of(&spec.file);
    json!({
        "bom_present": spec.bom_present,
        "bytes": spec.bytes.len(),
        "encoding": spec.encoding,
        "extension": extension,
        "file": spec.file,
        "foreign": extension != "notes",
        "line_ending": spec.line_ending,
        "note": spec.note,
        "sha256_hex": sha256_hex(&spec.bytes),
        "trailing_newline": spec.trailing_newline,
    })
}

/// The manifest document the generator writes — and the verifier rebuilds in
/// memory to diff against the one on disk.
fn manifest_document(specs: &[Spec]) -> Value {
    let files: Vec<Value> = specs.iter().map(manifest_row).collect();
    json!({
        "ansi_codepage": ANSI_CODEPAGE,
        "count": files.len(),
        "files": files,
    })
}

/// Write `bytes` to `path` so a concurrent reader never observes a mixture:
/// the payload goes to `<path>.new` in the same directory (exclusive create),
/// is flushed and fsynced, the handle is closed, and the finished side file is
/// renamed over `path`. A same-volume rename is atomic on NTFS and POSIX, so
/// a reader sees the previous bytes or the complete new ones - never a
/// partial write. On any failure the side file is removed: the repo must not
/// accumulate `.new` files, and the fixtures verifier would rightly report a
/// leftover one as an unlisted file. A `.new` left behind by a killed process
/// is stale, has no readers, and is removed once before the exclusive create
/// is retried - that is crash recovery, not a retry of a racing write.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut side_name = path.as_os_str().to_owned();
    side_name.push(".new");
    let side = PathBuf::from(side_name);
    let outcome = (|| -> std::io::Result<()> {
        let mut file = match fs::File::options().write(true).create_new(true).open(&side) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::remove_file(&side)?;
                fs::File::options()
                    .write(true)
                    .create_new(true)
                    .open(&side)?
            }
            Err(e) => return Err(e),
        };
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&side, path)?;
        Ok(())
    })();
    match outcome {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&side);
            Err(format!("cannot write {}: {e}", path.display()))
        }
    }
}

/// Write every fixture and the manifest into dir. Deterministic and
/// idempotent: the same bytes every run, no timestamps, sorted output. Every
/// file lands atomically (see [`atomic_write`]), and the manifest is written
/// LAST by the same rule: a complete manifest implies complete fixtures,
/// which is why the verifier can trust a manifest it can parse.
fn generate_tree(dir: &Path) -> Result<usize, String> {
    let specs = all_specs()?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for spec in &specs {
        atomic_write(&dir.join(&spec.file), &spec.bytes)?;
    }
    let manifest = serde_json::to_string_pretty(&manifest_document(&specs))
        .map_err(|e| format!("cannot serialize manifest: {e}"))?;
    let mut manifest = manifest;
    manifest.push('\n');
    atomic_write(&dir.join(MANIFEST_NAME), manifest.as_bytes())?;
    Ok(specs.len())
}

/// Entry point for "cargo xtask fixtures generate".
pub fn run_generate() -> i32 {
    match command_dir() {
        Ok(dir) => match generate_tree(&dir) {
            Ok(count) => {
                println!(
                    "fixtures generate: wrote {count} fixtures + {MANIFEST_NAME} to {}",
                    dir.display()
                );
                0
            }
            Err(e) => {
                eprintln!("fixtures generate: {e}");
                2
            }
        },
        Err(code) => code,
    }
}

/// What verify checks a file against: the generator's expectation, not the
/// manifest's self-description.
struct Entry {
    file: String,
    sha256_hex: String,
    bytes: usize,
}

fn expected_entries(specs: &[Spec]) -> Vec<Entry> {
    specs
        .iter()
        .map(|s| Entry {
            file: s.file.clone(),
            sha256_hex: sha256_hex(&s.bytes),
            bytes: s.bytes.len(),
        })
        .collect()
}

fn show(value: Option<&Value>) -> String {
    value
        .map(Value::to_string)
        .unwrap_or_else(|| "absent".to_string())
}

/// Layer (a): the manifest on disk must BE the generator's manifest. Rows
/// present exactly once, count == rows == generator count, ansi_codepage
/// intact, and every metadata field equal — a manifest row that is merely
/// self-consistent (bytes and hash edited together) still differs from what
/// the generator produces.
fn verify_manifest_against_generator(disk: &Value, specs: &[Spec]) -> Vec<String> {
    let mut violations = Vec::new();
    let expected_count = specs.len() as u64;
    if disk.get("ansi_codepage") != Some(&json!(ANSI_CODEPAGE)) {
        violations.push(format!(
            "manifest ansi_codepage is {}, generator says {ANSI_CODEPAGE} — the manifest is generator-owned",
            show(disk.get("ansi_codepage"))
        ));
    }
    let disk_count = disk.get("count").and_then(Value::as_u64);
    match disk_count {
        Some(c) if c == expected_count => {}
        Some(c) => violations.push(format!(
            "manifest count is {c}, generator says {expected_count} — the manifest is generator-owned"
        )),
        None => violations.push("manifest count is missing".to_string()),
    }
    let Some(rows) = disk.get("files").and_then(Value::as_array) else {
        violations.push("manifest has no 'files' array".to_string());
        return violations;
    };
    if let Some(c) = disk_count {
        if rows.len() as u64 != c {
            violations.push(format!(
                "manifest count is {c} but 'files' has {} rows",
                rows.len()
            ));
        }
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let name = row
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("<row without a file field>");
        let count = seen.entry(name).or_insert(0);
        *count += 1;
        if *count == 2 {
            violations.push(format!(
                "duplicate manifest row for {name} (extra at row {i})"
            ));
        }
        if !specs.iter().any(|s| s.file == name) {
            violations.push(format!(
                "manifest row the generator does not produce: {name} (row {i})"
            ));
        }
    }
    for spec in specs {
        let matching: Vec<&Value> = rows
            .iter()
            .filter(|r| r.get("file").and_then(Value::as_str) == Some(spec.file.as_str()))
            .collect();
        if matching.is_empty() {
            violations.push(format!(
                "manifest row missing: {} — the manifest must be regenerated, not edited",
                spec.file
            ));
            continue;
        }
        if matching.len() > 1 {
            continue; // the duplicate is already reported above
        }
        let expected = manifest_row(spec);
        let disk_row = matching[0];
        if disk_row == &expected {
            continue;
        }
        for key in [
            "bom_present",
            "bytes",
            "encoding",
            "extension",
            "file",
            "foreign",
            "line_ending",
            "note",
            "sha256_hex",
            "trailing_newline",
        ] {
            if disk_row.get(key) != expected.get(key) {
                violations.push(format!(
                    "manifest row {}: field {key} is {}, generator says {}",
                    spec.file,
                    show(disk_row.get(key)),
                    show(expected.get(key))
                ));
            }
        }
        if let Some(object) = disk_row.as_object() {
            for key in object.keys() {
                if !expected.as_object().is_some_and(|e| e.contains_key(key)) {
                    violations.push(format!(
                        "manifest row {} has a field the generator does not write: {key}",
                        spec.file
                    ));
                }
            }
        }
    }
    violations
}

/// The verifier proper: expected rows vs the directory on disk. Returns one
/// human-readable violation per problem, in a deterministic order.
fn verify_dir(dir: &Path, entries: &[Entry]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut present: BTreeSet<String> = BTreeSet::new();
    match fs::read_dir(dir) {
        Ok(read) => {
            for item in read {
                let entry = match item {
                    Ok(entry) => entry,
                    Err(e) => {
                        violations.push(format!("fixtures dir entry unreadable: {e}"));
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().to_string();
                if name == MANIFEST_NAME {
                    continue;
                }
                if entry.path().is_dir() {
                    violations.push(format!("unexpected directory in fixtures: {name}"));
                    continue;
                }
                present.insert(name);
            }
        }
        Err(e) => violations.push(format!("fixtures dir unreadable: {e}")),
    }
    for entry in entries {
        if !present.contains(&entry.file) {
            violations.push(format!("missing fixture: {}", entry.file));
            continue;
        }
        let actual = match fs::read(dir.join(&entry.file)) {
            Ok(bytes) => bytes,
            Err(e) => {
                violations.push(format!("fixture {} unreadable: {e}", entry.file));
                continue;
            }
        };
        if actual.len() != entry.bytes {
            violations.push(format!(
                "size mismatch: {} is {} bytes, manifest says {}",
                entry.file,
                actual.len(),
                entry.bytes
            ));
        }
        let actual_hex = sha256_hex(&actual);
        if actual_hex != entry.sha256_hex {
            violations.push(format!(
                "hash mismatch: {} hashes to {actual_hex}, manifest says {}",
                entry.file, entry.sha256_hex
            ));
        }
    }
    for name in &present {
        if !entries.iter().any(|entry| entry.file == *name) {
            violations.push(format!("unlisted file in fixtures dir: {name}"));
        }
    }
    violations
}

/// Layer (b): byte-compare the repo tree against a fresh generation in a
/// tempdir. Closes the hand-edit-plus-rehash hole: even a fixture whose
/// manifest row was edited to match cannot survive a regeneration diff.
fn compare_tree_against_generator(dir: &Path, generated: &Path, specs: &[Spec]) -> Vec<String> {
    let mut violations = Vec::new();
    for spec in specs {
        match (
            fs::read(dir.join(&spec.file)),
            fs::read(generated.join(&spec.file)),
        ) {
            (Ok(repo), Ok(fresh)) => {
                if repo != fresh {
                    violations.push(format!(
                        "{} differs from generator output (hand-edited?) — run: cargo xtask fixtures generate",
                        spec.file
                    ));
                }
            }
            (Err(e), _) => violations.push(format!("fixture {} unreadable: {e}", spec.file)),
            (_, Err(e)) => {
                violations.push(format!("generator could not produce {}: {e}", spec.file))
            }
        }
    }
    match (
        fs::read(dir.join(MANIFEST_NAME)),
        fs::read(generated.join(MANIFEST_NAME)),
    ) {
        (Ok(repo), Ok(fresh)) => {
            if repo != fresh {
                violations.push(format!(
                    "{MANIFEST_NAME} differs from generator output — the manifest is generator-owned; run: cargo xtask fixtures generate"
                ));
            }
        }
        (Err(e), _) => violations.push(format!("{MANIFEST_NAME} unreadable: {e}")),
        (_, Err(e)) => violations.push(format!("generator could not write its manifest: {e}")),
    }
    violations
}

fn fresh_tempdir(tag: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("xtask-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot create tempdir {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Entry point for "cargo xtask fixtures verify [args]". No cargo, no
/// network. The generator comparison (the reviewer's --against-generator) is
/// the DEFAULT and always runs; the flag is accepted so CI can ask for it
/// explicitly. Exit 0 clean, 1 violation, 2 the check itself could not run.
pub fn run_verify(rest: &[String]) -> i32 {
    for arg in rest {
        if arg != "--against-generator" {
            eprintln!("fixtures verify: unknown argument '{arg}'");
            return 2;
        }
    }
    let dir = match command_dir() {
        Ok(dir) => dir,
        Err(code) => return code,
    };
    let specs = match all_specs() {
        Ok(specs) => specs,
        Err(e) => {
            eprintln!("fixtures verify: {e}");
            return 2;
        }
    };
    let mut violations: Vec<String> = Vec::new();

    // Layer (a): manifest on disk vs the manifest the generator would write.
    match fs::read(dir.join(MANIFEST_NAME)) {
        Err(e) => violations.push(format!(
            "{MANIFEST_NAME} missing or unreadable: {e} — run: cargo xtask fixtures generate"
        )),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Err(e) => violations.push(format!("{MANIFEST_NAME} is not valid JSON: {e}")),
            Ok(disk) => violations.extend(verify_manifest_against_generator(&disk, &specs)),
        },
    }

    // Every file's bytes vs the generator's expected hash.
    violations.extend(verify_dir(&dir, &expected_entries(&specs)));

    // Layer (b): regenerate into a tempdir (never into the repo) and
    // byte-compare the whole tree, manifest included.
    let generated = match fresh_tempdir("verify-gen") {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("fixtures verify: {e}");
            return 2;
        }
    };
    match generate_tree(&generated) {
        Ok(_) => violations.extend(compare_tree_against_generator(&dir, &generated, &specs)),
        Err(e) => {
            eprintln!("fixtures verify: generator failed in tempdir: {e}");
            let _ = fs::remove_dir_all(&generated);
            return 2;
        }
    }
    let _ = fs::remove_dir_all(&generated);

    for v in &violations {
        println!("FIXTURES VIOLATION: {v}");
    }
    println!(
        "fixtures verify: {} files, {} violations",
        specs.len(),
        violations.len()
    );
    if violations.is_empty() { 0 } else { 1 }
}

/// Locate crates/core/tests/fixtures under the workspace root found from the
/// caller's CWD, or return the process exit code to use (2).
fn command_dir() -> Result<std::path::PathBuf, i32> {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fixtures: cannot read the current directory: {e}");
            return Err(2);
        }
    };
    match crate::find_workspace_root(&cwd) {
        Ok(root) => Ok(root.join(FIXTURES_REL)),
        Err(e) => {
            eprintln!("fixtures: {e}");
            Err(2)
        }
    }
}

/// SHA-256 (FIPS 180-4), hex-encoded. Hand-rolled on purpose: content
/// addressing of fixture bytes, not a security primitive, and no new
/// dependency is warranted for it (D31). Proven against the standard vectors
/// below and differential-tested by review against Node and OpenSSL.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut hex = String::with_capacity(64);
    for word in h {
        for byte in word.to_be_bytes() {
            hex.push_str(&format!("{byte:02x}"));
        }
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn sha_matches(data: &[u8], expected: &str) {
        assert_eq!(sha256_hex(data), expected);
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        sha_matches(
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        sha_matches(
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
        // The 56-byte vector: exercises the exact one-block padding boundary.
        sha_matches(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        );
    }

    fn tempdir(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("xtask-fixtures-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    fn read_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
            .expect("read tree")
            .map(|entry| entry.expect("dir entry"))
            .filter(|entry| entry.file_name().to_string_lossy() != MANIFEST_NAME)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let bytes = fs::read(entry.path()).expect("read file");
                (name, bytes)
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    fn specs_or_die() -> Vec<Spec> {
        all_specs().expect("spec tables must build")
    }

    #[test]
    fn generate_is_idempotent_across_two_tempdirs() {
        let a = tempdir("idem-a");
        let b = tempdir("idem-b");
        let count_a = generate_tree(&a).expect("generate a");
        let count_b = generate_tree(&b).expect("generate b");
        assert_eq!(count_a, count_b);
        let manifest_a = fs::read(a.join(MANIFEST_NAME)).expect("manifest a");
        let manifest_b = fs::read(b.join(MANIFEST_NAME)).expect("manifest b");
        assert_eq!(manifest_a, manifest_b, "manifests must be byte-identical");
        assert_eq!(
            read_tree(&a),
            read_tree(&b),
            "every file must be byte-identical"
        );
    }

    /// PROOF, not a comment: while the generator runs many rounds into a
    /// tempdir, a reader thread hammers every file with fs::read. Under the old
    /// truncate-in-place writer a racing read saw short or partial bytes (the
    /// tester measured 432 silent torn reads and 12 throwing ones in 3000
    /// rounds); under atomic rename every successful read is byte-identical to
    /// the expected content, and the only legal failure is NotFound before
    /// first creation. Coverage note: the read-side invariant is asserted
    /// unconditionally - this test cannot fail spuriously on the atomic
    /// writer, it WOULD have failed on the old one, and it does not claim to
    /// prove NTFS rename atomicity itself, only that our writer relies on it
    /// correctly. The assertion that reads > 0 keeps it from passing vacuously.
    #[test]
    fn a_concurrent_reader_never_sees_torn_bytes() {
        let dir = tempdir("torn");
        let specs = specs_or_die();
        let mut expected: Vec<(String, Vec<u8>)> = specs
            .iter()
            .map(|s| (s.file.clone(), s.bytes.clone()))
            .collect();
        let mut manifest_bytes = serde_json::to_string_pretty(&manifest_document(&specs))
            .expect("manifest serializes")
            .into_bytes();
        // The generator terminates the manifest with a newline; the expectation
        // must include it or every manifest read would "mismatch".
        manifest_bytes.push(b'\n');
        expected.push((MANIFEST_NAME.to_string(), manifest_bytes));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_reader = Arc::clone(&stop);
        let dir_for_reader = dir.clone();
        let reader = std::thread::spawn(move || {
            let mut violations = Vec::new();
            let mut reads = 0usize;
            let mut missing = 0usize;
            while !stop_reader.load(Ordering::Relaxed) {
                for (name, want) in &expected {
                    match fs::read(dir_for_reader.join(name)) {
                        Ok(bytes) => {
                            reads += 1;
                            if bytes != *want {
                                violations.push(format!(
                                    "torn read: {} is {} bytes, expected {}",
                                    name,
                                    bytes.len(),
                                    want.len()
                                ));
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            missing += 1; // legal only before first creation
                        }
                        Err(e) => violations.push(format!("read error: {name}: {e}")),
                    }
                }
            }
            (violations, reads, missing)
        });
        for _ in 0..12 {
            generate_tree(&dir).expect("generate round");
        }
        stop.store(true, Ordering::Relaxed);
        let (violations, reads, missing) = reader.join().expect("reader thread");
        assert!(
            reads > 0,
            "the reader must have observed real bytes, not only absence"
        );
        assert!(
            violations.is_empty(),
            "torn or wrong reads under a concurrent generator: {violations:?}"
        );
        let _ = missing; // counted for the record; absences are legal pre-creation
    }

    #[test]
    fn manifest_states_the_count_and_is_sorted_and_forward_slashed() {
        let specs = specs_or_die();
        let doc = manifest_document(&specs);
        assert_eq!(
            doc.get("count").and_then(Value::as_u64),
            Some(specs.len() as u64),
            "the manifest must state the count"
        );
        let files: Vec<String> = doc
            .get("files")
            .and_then(Value::as_array)
            .expect("files array")
            .iter()
            .filter_map(|e| e.get("file").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted, "manifest entries must be sorted by file");
        assert!(
            files.iter().all(|f| !f.contains('\\')),
            "no backslash paths"
        );
        let unique: BTreeSet<&String> = files.iter().collect();
        assert_eq!(unique.len(), files.len(), "no duplicate file names");
    }

    /// MINOR 5/6: preserve_order is in the dependency graph, so manifest keys
    /// are INSERTION-ordered; the json! literals are kept alphabetical on
    /// purpose and this test pins that, whichever feature resolution wins.
    #[test]
    fn manifest_keys_are_alphabetical_regardless_of_feature_flags() {
        let doc = manifest_document(&specs_or_die());
        let object = doc.as_object().expect("manifest object");
        let top: Vec<&String> = object.keys().collect();
        let mut top_sorted = top.clone();
        top_sorted.sort();
        assert_eq!(
            top, top_sorted,
            "top-level manifest keys must be alphabetical"
        );
        for row in object
            .get("files")
            .and_then(Value::as_array)
            .expect("files")
        {
            let keys: Vec<&String> = row.as_object().expect("row object").keys().collect();
            let mut sorted_keys = keys.clone();
            sorted_keys.sort();
            assert_eq!(keys, sorted_keys, "manifest row keys must be alphabetical");
        }
    }

    /// BLOCKER 1, as a permanent guard: a nonl fixture must not end in ANY
    /// line-terminator byte, per the encoding's byte order. The UTF-16 forms
    /// check the two-byte units, the single-byte encodings the raw byte. This
    /// is the test that would have caught utf16le__crlf__bom__nonl.json
    /// ending in a lone CR.
    #[test]
    fn nonl_fixtures_end_in_no_terminator_byte_in_any_encoding() {
        let specs = specs_or_die();
        for spec in &specs {
            let is_nonl = spec.file.ends_with("__nonl.txt") || spec.file.ends_with("__nonl.json");
            if !is_nonl {
                continue;
            }
            match spec.encoding {
                "utf16le" => {
                    assert!(
                        !spec.bytes.ends_with(&[0x0A, 0x00])
                            && !spec.bytes.ends_with(&[0x0D, 0x00]),
                        "{} ends in a line terminator: {:?}",
                        spec.file,
                        &spec.bytes[spec.bytes.len().saturating_sub(4)..]
                    );
                }
                "utf16be" => {
                    assert!(
                        !spec.bytes.ends_with(&[0x00, 0x0A])
                            && !spec.bytes.ends_with(&[0x00, 0x0D]),
                        "{} ends in a line terminator: {:?}",
                        spec.file,
                        &spec.bytes[spec.bytes.len().saturating_sub(4)..]
                    );
                }
                _ => {
                    let last = *spec.bytes.last().expect("a nonl fixture is never empty");
                    assert!(
                        last != 0x0A && last != 0x0D,
                        "{} ends in a line terminator byte 0x{last:02X}",
                        spec.file
                    );
                }
            }
        }
        for enc in ["utf8", "utf8bom", "utf16le", "utf16be", "ansi1252"] {
            assert!(
                specs.iter().any(|s| s.encoding == enc
                    && (s.file.ends_with("__nonl.txt") || s.file.ends_with("__nonl.json"))),
                "{enc} has no nonl fixture for the guard to check"
            );
        }
    }

    /// Mini fixtures for the verifier poison tests: two files plus a manifest.
    fn write_mini(dir: &Path) -> Vec<Entry> {
        fs::write(dir.join("a.txt"), b"hello\n").expect("write a");
        fs::write(dir.join("b.bin"), [0x00u8, 0xFF, 0x7F]).expect("write b");
        let manifest = json!({
            "count": 2,
            "files": [
                { "file": "a.txt", "sha256_hex": sha256_hex(b"hello\n"), "bytes": 6 },
                { "file": "b.bin", "sha256_hex": sha256_hex([0x00u8, 0xFF, 0x7F].as_slice()), "bytes": 3 },
            ],
        });
        fs::write(
            dir.join(MANIFEST_NAME),
            serde_json::to_string_pretty(&manifest).expect("serialize mini manifest"),
        )
        .expect("write mini manifest");
        vec![
            Entry {
                file: "a.txt".to_string(),
                sha256_hex: sha256_hex(b"hello\n"),
                bytes: 6,
            },
            Entry {
                file: "b.bin".to_string(),
                sha256_hex: sha256_hex([0x00u8, 0xFF, 0x7F].as_slice()),
                bytes: 3,
            },
        ]
    }

    #[test]
    fn a_clean_mini_dir_passes() {
        let dir = tempdir("clean");
        let entries = write_mini(&dir);
        assert!(verify_dir(&dir, &entries).is_empty());
    }

    #[test]
    fn a_flipped_byte_is_reported_as_a_hash_mismatch() {
        let dir = tempdir("flip");
        let entries = write_mini(&dir);
        fs::write(dir.join("a.txt"), b"hellp\n").expect("poison a");
        let violations = verify_dir(&dir, &entries);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("hash mismatch"), "{violations:?}");
        assert!(violations[0].contains("a.txt"), "{violations:?}");
    }

    #[test]
    fn a_stray_file_is_reported_as_unlisted() {
        let dir = tempdir("stray");
        let entries = write_mini(&dir);
        fs::write(dir.join("c.txt"), b"sneaky\n").expect("write stray");
        let violations = verify_dir(&dir, &entries);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("unlisted"), "{violations:?}");
        assert!(violations[0].contains("c.txt"), "{violations:?}");
    }

    #[test]
    fn a_deleted_file_is_reported_as_missing() {
        let dir = tempdir("missing");
        let entries = write_mini(&dir);
        fs::remove_file(dir.join("b.bin")).expect("delete b");
        let violations = verify_dir(&dir, &entries);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("missing"), "{violations:?}");
        assert!(violations[0].contains("b.bin"), "{violations:?}");
    }

    /// The generated set itself must pass its own verifier — all layers.
    #[test]
    fn the_generated_tree_verifies_clean() {
        let dir = tempdir("selfcheck");
        let count = generate_tree(&dir).expect("generate");
        let specs = specs_or_die();
        assert_eq!(count, specs.len());
        assert!(verify_dir(&dir, &expected_entries(&specs)).is_empty());
        let generated = tempdir("selfcheck-gen");
        generate_tree(&generated).expect("generate again");
        assert!(compare_tree_against_generator(&dir, &generated, &specs).is_empty());
    }

    /// The proof the brief asks for, pinned as a test: the UTF-16 fixtures
    /// are two bytes per character plus the 2-byte BOM, the ansi1252 file is
    /// one byte per character, and the nonl variants really do not end in a
    /// newline byte.
    #[test]
    fn byte_length_invariants_hold() {
        let specs = specs_or_die();
        let find = |name: &str| {
            specs
                .iter()
                .find(|s| s.file == name)
                .unwrap_or_else(|| panic!("fixture {name} missing"))
        };
        let utf8 = find("utf8__lf__nobom__nl.txt");
        let utf16 = find("utf16le__lf__bom__nl.txt");
        assert_eq!(utf8.bytes.len(), 57, "56 chars + one extra byte for é");
        assert_eq!(
            utf16.bytes.len(),
            114,
            "2 bytes per char (56 chars) + the 2-byte BOM"
        );
        assert_eq!(
            find("ansi1252__lf__nobom__nl.txt").bytes.len(),
            56,
            "one byte per char: é is a single 0xE9 byte in CP1252"
        );
        let nonl_utf8 = find("utf8__lf__nobom__nonl.txt");
        let nonl_utf16 = find("utf16le__crlf__bom__nonl.txt");
        assert!(!nonl_utf8.bytes.ends_with(b"\n"));
        assert!(!nonl_utf16.bytes.ends_with(&[0x0A, 0x00]));
    }

    // ---- mutation tests of the verifier itself (layer a) ----

    fn mutated_disk(mutate: impl FnOnce(&mut Value)) -> (Value, Vec<Spec>) {
        let specs = specs_or_die();
        let mut disk = manifest_document(&specs);
        mutate(&mut disk);
        (disk, specs)
    }

    fn rows_mut(disk: &mut Value) -> &mut Vec<Value> {
        disk.get_mut("files")
            .and_then(Value::as_array_mut)
            .expect("files array")
    }

    #[test]
    fn a_deleted_manifest_row_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            rows_mut(d).remove(0);
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("manifest row missing")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.contains("'files' has")),
            "count-vs-rows must also fire: {violations:?}"
        );
    }

    #[test]
    fn a_duplicated_manifest_row_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            let first = rows_mut(d)[0].clone();
            rows_mut(d).push(first);
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("duplicate manifest row")),
            "{violations:?}"
        );
    }

    #[test]
    fn a_flipped_count_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            d.as_object_mut()
                .expect("manifest object")
                .insert("count".to_string(), json!(5));
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("manifest count is 5, generator says 28")),
            "{violations:?}"
        );
    }

    #[test]
    fn a_flipped_metadata_field_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            rows_mut(d)[0]
                .as_object_mut()
                .expect("row object")
                .insert("encoding".to_string(), json!("utf16le"));
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations.iter().any(|v| v.contains("field encoding is")),
            "{violations:?}"
        );
    }

    /// THE hand-edit-plus-rehash attack: the row is made self-consistent (the
    /// hash matches the tampered bytes) — and the verifier still fails,
    /// because the row must match what the GENERATOR produces.
    #[test]
    fn a_hand_edited_row_with_recomputed_hash_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            let rows = rows_mut(d);
            let row = rows
                .iter_mut()
                .find(|r| r.get("file").and_then(Value::as_str) == Some("utf8__lf__nobom__nl.txt"))
                .expect("row exists");
            let object = row.as_object_mut().expect("row object");
            object.insert("bytes".to_string(), json!(999));
            object.insert("sha256_hex".to_string(), json!(sha256_hex(b"tampered")));
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations.iter().any(|v| v.contains("field bytes is 999")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.contains("field sha256_hex is")),
            "{violations:?}"
        );
    }

    #[test]
    fn an_extra_manifest_row_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            rows_mut(d).push(json!({
                "file": "sneaky.txt",
                "sha256_hex": "00",
                "bytes": 0,
            }));
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("the generator does not produce: sneaky.txt")),
            "{violations:?}"
        );
    }

    #[test]
    fn a_flipped_ansi_codepage_is_reported() {
        let (disk, specs) = mutated_disk(|d| {
            d.as_object_mut()
                .expect("manifest object")
                .insert("ansi_codepage".to_string(), json!(850));
        });
        let violations = verify_manifest_against_generator(&disk, &specs);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("ansi_codepage is 850")),
            "{violations:?}"
        );
    }

    /// Layer (b): a hand-edited file is caught by the regeneration diff even
    /// if every manifest row were made to agree with it.
    #[test]
    fn generator_comparison_catches_a_hand_edited_file() {
        let repo = tempdir("cmp-repo");
        let generated = tempdir("cmp-gen");
        generate_tree(&repo).expect("generate repo copy");
        generate_tree(&generated).expect("generate fresh");
        fs::write(repo.join("utf8__lf__nobom__nl.txt"), b"tampered").expect("hand-edit");
        let violations = compare_tree_against_generator(&repo, &generated, &specs_or_die());
        assert!(
            violations
                .iter()
                .any(|v| v.contains("utf8__lf__nobom__nl.txt")
                    && v.contains("differs from generator")),
            "{violations:?}"
        );
    }
}
