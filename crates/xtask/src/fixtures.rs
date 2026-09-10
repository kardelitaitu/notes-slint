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
//!   (a pure-ASCII CP1252 file is indistinguishable from UTF-8) while every
//!   character still occupies one or two UTF-8 bytes and exactly two UTF-16
//!   bytes, so a UTF-16 fixture is precisely twice the UTF-8 length plus its
//!   2-byte BOM.
//! * The fixed product crosses encoding x line-ending x trailing-newline:
//!   5 encodings x 2 EOLs x 2 trailings = 20 files. The BOM field of the D8
//!   name is determined by the encoding (utf8bom/utf16le/utf16be always carry
//!   a BOM; utf8/ansi1252 never do), so there are no impossible combinations
//!   to skip: a BOM-less utf16le is not part of this product.
//! * Hand-picked edge cases (empty, lone CR, BOM-only, mixed EOLs, a
//!   character outside CP1252, the documented frontmatter block, and two
//!   foreign .json files) are listed verbatim below.
//! * manifest.json states the count and one entry per file with the SAME
//!   vocabulary as core::encoding's Detected / api::FileMeta: encodings
//!   utf8, utf8bom, utf16le, utf16be, ansi1252; line endings lf, crlf (the
//!   dominant rule in core::encoding: lone CR and LF/CRLF mixes report lf).
//!   Hashes are computed by this file's own SHA-256, never taken from Git.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

const FIXTURES_REL: &str = "crates/core/tests/fixtures";
const MANIFEST_NAME: &str = "manifest.json";

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

/// Apply the EOL rule to the base text, then the trailing rule. "nonl" strips
/// the whole final terminator (\r\n for CRLF files, not just the \n).
fn product_text(crlf: bool, keep_nl: bool) -> String {
    let mut text = if crlf {
        BASE_TEXT.replace('\n', "\r\n")
    } else {
        BASE_TEXT.to_string()
    };
    if !keep_nl {
        if crlf {
            if text.ends_with("\r\n") {
                text.truncate(text.len() - 2);
            }
        } else if text.ends_with('\n') {
            text.pop();
        }
    }
    text
}

fn product_specs() -> Result<Vec<Spec>, String> {
    let mut specs = Vec::new();
    for enc in ALL_ENCODINGS {
        for (crlf, eol_name) in [(false, "lf"), (true, "crlf")] {
            for (keep_nl, nl_name) in [(true, "nl"), (false, "nonl")] {
                let text = product_text(crlf, keep_nl);
                specs.push(Spec {
                    file: format!(
                        "{}__{}__{}__{}.md",
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
            file: "edge__outside-ansi1252.md".to_string(),
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
            bytes: Enc::Utf16Le.encode(&product_json_text(true, false)).expect("base json is cp1252-free"),
            encoding: "utf16le",
            line_ending: "crlf",
            trailing_newline: false,
            bom_present: true,
            note: "foreign extension, UTF-16LE: the same no-rewrite rule for a file core detects as UTF-16".to_string(),
        },
    ]
}

/// The base JSON under the EOL/trailing rules, for the foreign .json rows.
fn product_json_text(crlf: bool, keep_nl: bool) -> String {
    let mut text = if crlf {
        BASE_JSON.replace('\n', "\r\n")
    } else {
        BASE_JSON.to_string()
    };
    if !keep_nl && text.ends_with('\n') {
        text.pop();
    }
    text
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

/// The manifest document: count + one entry per file, keys emitted in
/// serde_json's deterministic (sorted) order, forward-slash file names.
fn manifest_document(specs: &[Spec]) -> Value {
    let files: Vec<Value> = specs
        .iter()
        .map(|s| {
            let extension = extension_of(&s.file);
            json!({
                "bom_present": s.bom_present,
                "bytes": s.bytes.len(),
                "encoding": s.encoding,
                "extension": extension,
                "file": s.file,
                "foreign": extension != "notes",
                "line_ending": s.line_ending,
                "note": s.note,
                "sha256_hex": sha256_hex(&s.bytes),
                "trailing_newline": s.trailing_newline,
            })
        })
        .collect();
    json!({ "count": files.len(), "files": files })
}

/// Write every fixture and the manifest into dir. Deterministic and
/// idempotent: the same bytes every run, no timestamps, sorted output.
fn generate_tree(dir: &Path) -> Result<usize, String> {
    let specs = all_specs()?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for spec in &specs {
        fs::write(dir.join(&spec.file), &spec.bytes)
            .map_err(|e| format!("cannot write {}: {e}", spec.file))?;
    }
    let manifest = serde_json::to_string_pretty(&manifest_document(&specs))
        .map_err(|e| format!("cannot serialize manifest: {e}"))?;
    fs::write(dir.join(MANIFEST_NAME), manifest + "\n")
        .map_err(|e| format!("cannot write {MANIFEST_NAME}: {e}"))?;
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

/// One parsed manifest row: what verify actually checks.
struct Entry {
    file: String,
    sha256_hex: String,
    bytes: usize,
}

fn parse_manifest(value: &Value) -> Result<Vec<Entry>, String> {
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest: missing 'files'".to_string())?;
    files
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let file = e
                .get("file")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("manifest: entry {i} has no file"))?
                .to_string();
            let sha256_hex = e
                .get("sha256_hex")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("manifest: entry {i} ({file}) has no sha256_hex"))?
                .to_string();
            let bytes = e
                .get("bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("manifest: entry {i} ({file}) has no byte count"))?
                as usize;
            Ok(Entry {
                file,
                sha256_hex,
                bytes,
            })
        })
        .collect()
}

/// The verifier proper: manifest rows vs the directory on disk. Returns one
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

/// Entry point for "cargo xtask fixtures verify". No cargo, no network.
pub fn run_verify() -> i32 {
    let dir = match command_dir() {
        Ok(dir) => dir,
        Err(code) => return code,
    };
    let manifest_path = dir.join(MANIFEST_NAME);
    let manifest_bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!(
                "fixtures verify: cannot read {}: {e} (run: cargo xtask fixtures generate)",
                manifest_path.display()
            );
            return 2;
        }
    };
    let manifest: Value = match serde_json::from_slice(&manifest_bytes) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("fixtures verify: {MANIFEST_NAME} is not valid JSON: {e}");
            return 2;
        }
    };
    let entries = match parse_manifest(&manifest) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("fixtures verify: {e}");
            return 2;
        }
    };
    let violations = verify_dir(&dir, &entries);
    for v in &violations {
        println!("FIXTURES VIOLATION: {v}");
    }
    println!(
        "fixtures verify: {} files, {} violations",
        entries.len(),
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
/// dependency is warranted for it. Proven against the standard vectors below.
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[test]
    fn manifest_states_the_count_and_is_sorted_and_forward_slashed() {
        let specs = all_specs().expect("specs");
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

    /// Mini fixtures for the poison tests: two files plus a manifest built by
    /// the same code path the generator uses.
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
        parse_manifest(&manifest).expect("parse mini manifest")
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

    /// The generated set itself must pass its own verifier — and the product
    /// must be the honest size: 20 product files + 8 documented edge cases.
    #[test]
    fn the_generated_tree_verifies_clean() {
        let dir = tempdir("selfcheck");
        let count = generate_tree(&dir).expect("generate");
        assert_eq!(count, 28);
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join(MANIFEST_NAME)).expect("manifest"))
                .expect("parse");
        let entries = parse_manifest(&manifest).expect("entries");
        assert_eq!(entries.len(), count);
        assert!(
            verify_dir(&dir, &entries).is_empty(),
            "generated tree must self-verify"
        );
    }

    /// The proof the brief asks for, pinned as a test: UTF-16 is exactly
    /// twice the UTF-8 length plus the 2-byte BOM for the base text, and the
    /// nonl variants really do not end in a newline byte.
    #[test]
    fn byte_length_invariants_hold() {
        let specs = all_specs().expect("specs");
        let find = |name: &str| {
            specs
                .iter()
                .find(|s| s.file == name)
                .unwrap_or_else(|| panic!("fixture {name} missing"))
        };
        let utf8 = find("utf8__lf__nobom__nl.md");
        let utf16 = find("utf16le__lf__bom__nl.md");
        assert_eq!(utf8.bytes.len(), 57, "56 chars + one extra byte for é");
        assert_eq!(
            utf16.bytes.len(),
            114,
            "2 bytes per char (56 chars) + the 2-byte BOM"
        );
        assert_eq!(
            find("ansi1252__lf__nobom__nl.md").bytes.len(),
            56,
            "one byte per char: é is a single 0xE9 byte in CP1252"
        );
        let nonl_utf8 = find("utf8__lf__nobom__nonl.md");
        let nonl_utf16 = find("utf16le__crlf__bom__nonl.md");
        assert!(!nonl_utf8.bytes.ends_with(b"\n"));
        assert!(!nonl_utf16.bytes.ends_with(&[0x0A, 0x00]));
    }
}
