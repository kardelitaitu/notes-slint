//! THE DO-NO-HARM GATE, FINISH. THE PORT LEG (M2 exit item 3).
//!
//! `crates/core/tests/roundtrip.rs` proves that core decodes, encodes and
//! atomic-writes every fixture in `crates/core/tests/fixtures/manifest.json`
//! byte-for-byte. It stops at core's front door. Nothing in `crates/api/tests`
//! ever read that corpus: the port's own byte-identity evidence was a handful of
//! hand-written literals (`b"foreign file\r\n"`, `b"line one\r\nline two"`, a
//! nine-byte CP1252 `caf\u{e9}`), so UTF-16 LE/BE, a BOM-only file, lone CR, mixed
//! EOLs, a zero-byte note, a front-matter note, non-BMP emoji and the `.json` and
//! `.notes` names had NO assertion on the leg a bridge actually drives.
//!
//! That leg is not free. Between `Command::Open` and the bytes on disk the port
//! translates core's verdict into `FileMeta` (D14's `Ansi(codepage)`), echoes a
//! document generation the engine owns (`epoch`), gates the write behind core's
//! fixed skip order (ADR-0001, D11), and - for `.notes` files only - strips the
//! front matter on the way in (`Engine::body_for_ui`) and rebuilds it on the way
//! out (`Engine::text_for_disk`). A defect in any of those five is invisible to
//! core's gate and shows up as a user's file quietly rewritten.
//!
//! THE ONE HONEST DEVIATION FROM "Open, then Flush", stated because it is a
//! behaviour and not a shortcut: ADR-0001 keeps a foreign file DISARMED, so a first
//! `Flush` on 25 of the 30 fixtures is refused and writes NOTHING. Asserting
//! byte-equality after that refusal would prove only that no I/O happened. So the
//! drive asserts the refusal first (and that the file is untouched by it), performs
//! the one explicit save the decision names as the arming act -
//! `Command::SaveAs` at the file's OWN path, which is what the live bridge puts
//! behind `ctrl-s` (`crates/bridge-gpui/src/menu.rs` maps `SaveAsFile`) - and THEN
//! flushes the exact text `Event::Loaded` handed out and judges the bytes. Every
//! fixture therefore ends with a real write on the autosave leg, and the 5
//! `.notes` fixtures additionally prove their FIRST flush saves outright.
//!
//! A fixture added to the manifest becomes a checked case with no edit here. The
//! manifest must exist, parse and be non-empty: a skipped gate looks identical to a
//! green one, so a missing or self-inconsistent manifest FAILS.
//!
//! The corpus is generator-owned and READ-ONLY from here: each case is copied into
//! a temp dir and the COPY is what the engine opens, and every drive ends by
//! re-reading the fixture itself to prove this file never wrote into it.
//!
//! Why the manifest is read by the small JSON scanner below rather than by
//! `serde_json`, which core's copy of this gate uses: `serde_json` is not a
//! dependency of `notes-api`, and this slice's fence is this one file. A scanner
//! that fails loudly on a shape it does not understand is the smaller cost.

/// The shared fake carries helpers the other host-driven files use (`set_answers`,
/// `moves`, `topmost`, `restore_reads`); this gate needs the answers and the call log
/// only, so the unused ones are silenced HERE rather than by editing the shared
/// support file.
#[allow(dead_code)]
#[path = "support/host_mock.rs"]
mod host_mock;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use host_mock::{Answers, Call, Host};
use notes_api::{
    Command, Encoding, Event, FileMeta, Gateway, LineEnding, Settings, SkipReason, StateDir,
    WindowHandle,
};

// notes-core and thiserror are dependencies of notes-api, not of this test
// target; naming them keeps `unused_crate_dependencies` honest.
use notes_core as _;
use thiserror as _;

/// Long enough that a loaded machine cannot flake; short enough that a hung
/// engine fails the run instead of hanging it forever.
const ANSWER: Duration = Duration::from_secs(5);

/// The ANSI code page this gate judges the corpus under - the same constant
/// core's gate declares, and it must agree with the manifest's own
/// `ansi_codepage` or a corpus authored for one page could be judged under
/// another in silence.
const GATE_CODEPAGE: u16 = 1252;

// ------------------------------------------------------------------ the corpus

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/api has a parent")
        .join("core")
        .join("tests")
        .join("fixtures")
}

/// One manifest entry, in exactly the vocabulary the generator writes. Unknown
/// encoding/EOL tokens are REFUSED by `encoding_from_str`/`line_ending_from_str`
/// rather than skipped: a manifest typo must not become a test that proves
/// nothing.
struct Entry {
    file: String,
    encoding: String,
    line_ending: String,
    trailing_newline: bool,
    bom_present: bool,
    foreign: bool,
    bytes: u64,
}

struct Manifest {
    ansi_codepage: u16,
    count: u64,
    entries: Vec<Entry>,
}

fn load_manifest() -> Result<Manifest, String> {
    let path = fixtures_dir().join("manifest.json");
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("the port's do-no-harm gate cannot run: {path:?} unreadable: {e}"))?;
    let root =
        parse_json(&text).map_err(|e| format!("manifest at {path:?} does not parse: {e}"))?;

    let codepage = as_u64(
        member(&root, "ansi_codepage", "the manifest")?,
        "ansi_codepage",
    )?;
    let count = as_u64(member(&root, "count", "the manifest")?, "count")?;
    let files = as_arr(member(&root, "files", "the manifest")?, "files")?;

    let mut entries = Vec::with_capacity(files.len());
    for (index, item) in files.iter().enumerate() {
        let at = format!("files[{index}]");
        entries.push(Entry {
            file: as_str(member(item, "file", &at)?, "file")?.to_owned(),
            encoding: as_str(member(item, "encoding", &at)?, "encoding")?.to_owned(),
            line_ending: as_str(member(item, "line_ending", &at)?, "line_ending")?.to_owned(),
            trailing_newline: as_bool(member(item, "trailing_newline", &at)?, "trailing_newline")?,
            bom_present: as_bool(member(item, "bom_present", &at)?, "bom_present")?,
            foreign: as_bool(member(item, "foreign", &at)?, "foreign")?,
            bytes: as_u64(member(item, "bytes", &at)?, "bytes")?,
        });
    }
    if entries.is_empty() {
        return Err(format!(
            "manifest at {path:?} lists no fixtures - the gate is empty"
        ));
    }
    if count as usize != entries.len() {
        return Err(format!(
            "manifest inconsistency: count = {count} but files lists {}",
            entries.len()
        ));
    }
    let codepage = u16::try_from(codepage)
        .map_err(|_| format!("ansi_codepage {codepage} does not fit a code page id"))?;
    if codepage != GATE_CODEPAGE {
        return Err(format!(
            "the fixture corpus declares ansi_codepage {codepage} but this gate judges it \
             under {GATE_CODEPAGE} - the two must agree"
        ));
    }
    Ok(Manifest {
        ansi_codepage: codepage,
        count,
        entries,
    })
}

fn encoding_from_str(token: &str, codepage: u16) -> Result<Encoding, String> {
    match token {
        "utf8" => Ok(Encoding::Utf8),
        "utf8bom" => Ok(Encoding::Utf8Bom),
        "utf16le" => Ok(Encoding::Utf16Le),
        "utf16be" => Ok(Encoding::Utf16Be),
        "ansi1252" => Ok(Encoding::Ansi(codepage)),
        other => Err(format!(
            "unknown encoding token {other:?} in the manifest - extend the mapping or fix it"
        )),
    }
}

fn line_ending_from_str(token: &str) -> Result<LineEnding, String> {
    match token {
        "lf" => Ok(LineEnding::Lf),
        "crlf" => Ok(LineEnding::CrLf),
        other => Err(format!(
            "unknown line_ending token {other:?} in the manifest - extend the mapping or fix it"
        )),
    }
}

/// A byte-level diff for the failure messages: a `String` comparison cannot see a
/// lost CR or a swapped BOM, so mismatches are reported as offset/hex pairs.
fn hex_diff(expected: &[u8], actual: &[u8]) -> String {
    let len = expected.len().max(actual.len());
    let diffs: Vec<usize> = (0..len)
        .filter(|&i| expected.get(i) != actual.get(i))
        .collect();
    let mut out = format!(
        "lengths: expected {} bytes vs actual {} bytes; {} differ\n",
        expected.len(),
        actual.len(),
        diffs.len()
    );
    for &i in diffs.iter().take(8) {
        let e = expected
            .get(i)
            .map(|b| format!("{b:02X}"))
            .unwrap_or_else(|| "--".to_owned());
        let a = actual
            .get(i)
            .map(|b| format!("{b:02X}"))
            .unwrap_or_else(|| "--".to_owned());
        out.push_str(&format!("  {i:#08x}: expected {e} vs actual {a}\n"));
    }
    if diffs.len() > 8 {
        out.push_str(&format!(
            "  ... and {} more differing bytes\n",
            diffs.len() - 8
        ));
    }
    out
}

// ------------------------------------------------------ a minimal JSON reader
//
// Enough of JSON to read the manifest honestly: objects, arrays, strings with
// escapes (surrogate pairs included), non-negative integers, true/false/null.
// Anything it does not understand is an Err, never a default - a silently
// mis-read manifest would turn this whole gate green for free.

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Int(u64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Scanner {
            bytes: src.as_bytes(),
            at: 0,
        }
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, want: u8) -> Result<(), String> {
        if self.peek() == Some(want) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!(
                "expected {:?} at byte {}",
                (want as char).to_string(),
                self.at
            ))
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(())
        } else {
            Err(format!("expected {word} at byte {}", self.at))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        let Some(first) = self.peek() else {
            return Err("the manifest ended in the middle of a value".to_owned());
        };
        match first {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => {
                self.literal("true")?;
                Ok(Json::Bool(true))
            }
            b'f' => {
                self.literal("false")?;
                Ok(Json::Bool(false))
            }
            b'n' => {
                self.literal("null")?;
                Ok(Json::Null)
            }
            b'0'..=b'9' => self.integer(),
            other => Err(format!("unexpected byte {other:#04X} at byte {}", self.at)),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.eat(b'{')?;
        let mut fields = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Obj(fields));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.eat(b':')?;
            fields.push((key, self.value()?));
            self.ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Obj(fields));
                }
                other => return Err(format!("expected , or }} in an object, got {other:?}")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.eat(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Arr(items));
                }
                other => return Err(format!("expected , or ] in an array, got {other:?}")),
            }
        }
    }

    fn integer(&mut self) -> Result<Json, String> {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|e| e.to_string())?;
        // A fraction or exponent would be a manifest this gate cannot read;
        // `bytes`, `count` and `ansi_codepage` are integers by construction.
        text.parse::<u64>()
            .map(Json::Int)
            .map_err(|e| format!("{text:?} at byte {start} is not an integer: {e}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.ws();
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err("unterminated string".to_owned());
            };
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                b'\\' => {
                    let Some(esc) = self.peek() else {
                        return Err("unterminated escape".to_owned());
                    };
                    self.at += 1;
                    match esc {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => self.escape_u(&mut out)?,
                        other => return Err(format!("unknown escape \\{}", other as char)),
                    }
                }
                _ => out.push(byte),
            }
        }
    }

    /// `\uXXXX`, with the surrogate pair a non-BMP emoji really is encoded as.
    /// The corpus is raw UTF-8, so this path is the belt-and-braces half of
    /// "fails loudly on a shape it does not understand".
    fn escape_u(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        let first = self.hex4()?;
        let ch = match first {
            0xD800..=0xDBFF => {
                if self.peek() != Some(b'\\') {
                    return Err("a lone high surrogate".to_owned());
                }
                self.at += 1;
                self.eat(b'u')?;
                let low = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&low) {
                    return Err("a surrogate pair without a low half".to_owned());
                }
                let point =
                    0x1_0000 + ((u32::from(first) - 0xD800) << 10) + u32::from(low) - 0xDC00;
                char::from_u32(point).ok_or_else(|| "surrogate pair is not a scalar".to_owned())?
            }
            0xDC00..=0xDFFF => return Err("a lone low surrogate".to_owned()),
            other => char::from_u32(u32::from(other))
                .ok_or_else(|| "escape is not a scalar".to_owned())?,
        };
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        Ok(())
    }

    fn hex4(&mut self) -> Result<u16, String> {
        if self.at + 4 > self.bytes.len() {
            return Err("a truncated \\u escape".to_owned());
        }
        let text =
            std::str::from_utf8(&self.bytes[self.at..self.at + 4]).map_err(|e| e.to_string())?;
        let value = u16::from_str_radix(text, 16)
            .map_err(|e| format!("{text:?} is not 4 hex digits: {e}"))?;
        self.at += 4;
        Ok(value)
    }
}

fn parse_json(src: &str) -> Result<Json, String> {
    let mut scanner = Scanner::new(src);
    let value = scanner.value()?;
    scanner.ws();
    if scanner.peek().is_some() {
        return Err("trailing content after the manifest's JSON value".to_owned());
    }
    Ok(value)
}

fn member<'j>(json: &'j Json, key: &str, at: &str) -> Result<&'j Json, String> {
    let Json::Obj(fields) = json else {
        return Err(format!("{at} is not a JSON object, so it has no {key:?}"));
    };
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("{at} has no {key:?} field - the manifest shape changed"))
}

fn as_str<'j>(json: &'j Json, key: &str) -> Result<&'j str, String> {
    match json {
        Json::Str(text) => Ok(text),
        other => Err(format!("{key:?} is not a string: {other:?}")),
    }
}

fn as_bool(json: &Json, key: &str) -> Result<bool, String> {
    match json {
        Json::Bool(value) => Ok(*value),
        other => Err(format!("{key:?} is not a boolean: {other:?}")),
    }
}

fn as_u64(json: &Json, key: &str) -> Result<u64, String> {
    match json {
        Json::Int(value) => Ok(*value),
        other => Err(format!("{key:?} is not an integer: {other:?}")),
    }
}

fn as_arr<'j>(json: &'j Json, key: &str) -> Result<&'j [Json], String> {
    match json {
        Json::Arr(items) => Ok(items),
        other => Err(format!("{key:?} is not an array: {other:?}")),
    }
}

// ---------------------------------------------------------------- the engine

/// One running port, its event wire, and the two temp dirs it owns: a private
/// `StateDir` (so no case can see another's `session.json`) and a SEPARATE
/// directory for the copy of the fixture, because a note the app did not create
/// lives nowhere near its state - and the corpus must never be the write target.
struct Port {
    gateway: Gateway,
    rx: Receiver<Event>,
    host: Host,
    notes: PathBuf,
    _state: tempfile::TempDir,
    _copies: tempfile::TempDir,
    /// The document generation the engine last announced, echoed back on every
    /// `Flush` exactly as a bridge's pump stores it from `Loaded`/`Rebound`.
    epoch: u64,
}

impl Port {
    /// The fake host is what lets the ANSI fixtures in through the honest door: a
    /// code page is a measured machine fact, and with no `HostFacts` an ANSI file
    /// is refused rather than guessed (D27). The same `Host` answers the geometry
    /// seam, so nothing here touches Win32.
    fn start(codepage: u16) -> Self {
        let state = tempfile::tempdir().expect("a temp state dir");
        let copies = tempfile::tempdir().expect("a temp dir for the fixture copies");
        let host = Host::with_answers(Answers {
            codepage,
            ..Answers::default()
        });
        let (gateway, rx) = Gateway::start_with_host(
            StateDir(state.path().to_path_buf()),
            Settings::default(),
            Some(Box::new(host.clone())),
            Some(Box::new(host.clone())),
        );
        // 5.5 step 3: the window exists before anything is asked of it, so the
        // engine is in the state a bridge leaves it in.
        let _ = gateway.send(Command::RegisterWindow {
            handle: WindowHandle(0x100),
        });
        Port {
            gateway,
            rx,
            host,
            notes: copies.path().to_path_buf(),
            _copies: copies,
            _state: state,
            epoch: 0,
        }
    }

    /// Lays a fixture's bytes down under its OWN name: the extension is part of
    /// the case, since `.notes` is what decides both arming and front matter.
    fn copy_of(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.notes.join(name);
        fs::write(&path, bytes).expect("copy the fixture into the temp dir");
        path
    }

    fn bytes_of(&self, path: &Path) -> Vec<u8> {
        fs::read(path).unwrap_or_default()
    }

    fn send(&self, command: Command) {
        assert!(
            self.gateway.send(command.clone()).is_ok(),
            "the engine must still be accepting commands: {command:?}"
        );
    }

    /// THE WIRE: every `Loaded`/`Rebound` that flows past updates the echoed
    /// generation, so a `Flush` can never be one bump out of step, and nothing is
    /// ever mirrored at send time.
    fn until<F>(&mut self, want: &str, f: F) -> Event
    where
        F: Fn(&Event) -> bool,
    {
        let mut seen = Vec::new();
        let deadline = Instant::now() + ANSWER;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(timeout) {
                Ok(event) => {
                    if let Event::Loaded { epoch, .. } | Event::Rebound { epoch, .. } = &event {
                        self.epoch = *epoch;
                    }
                    if f(&event) {
                        return event;
                    }
                    seen.push(event);
                }
                Err(err) => panic!("never saw {want}; saw {seen:?} first ({err:?})"),
            }
        }
    }

    fn loaded(&mut self, path: &Path) -> (String, FileMeta) {
        self.send(Command::Open {
            path: path.to_path_buf(),
        });
        match self.until(
            "Loaded",
            |ev| matches!(ev, Event::Loaded { path: p, .. } if p == path),
        ) {
            Event::Loaded { text, meta, .. } => (text, meta),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// A `Flush`, answered honestly: either it saved at a revision, or the engine
    /// said WHY nothing happened (ADR-0001: silence is forbidden). A
    /// `SaveFailed` here panics - a refused write is not a skip.
    fn flushed(&mut self, path: &Path, text: &str, revision: u64) -> Result<u64, SkipReason> {
        self.send(Command::Flush {
            text: text.to_owned(),
            revision,
            epoch: self.epoch,
        });
        match self.until("Saved or a skip", |ev| {
            matches!(
                ev,
                Event::Saved { .. } | Event::AutosaveSkipped { .. } | Event::SaveFailed { .. }
            )
        }) {
            Event::Saved {
                path: saved,
                revision,
            } => {
                assert_eq!(&saved, path, "Saved named a file other than the open one");
                Ok(revision)
            }
            Event::AutosaveSkipped { reason } => Err(reason),
            Event::SaveFailed { path, reason, .. } => {
                panic!("the flush to {} failed: {reason}", path.display())
            }
            other => panic!("expected Saved or a skip, got {other:?}"),
        }
    }

    /// The one explicit act ADR-0001 requires of a foreign file: `Saved` then
    /// `Rebound`, and the `Rebound` is what carries the new generation.
    fn saved_as(&mut self, path: &Path, text: &str, revision: u64) {
        self.send(Command::SaveAs {
            path: path.to_path_buf(),
            text: text.to_owned(),
            revision,
        });
        match self.until("a verdict on the Save As", |ev| match ev {
            Event::Saved { path: p, .. } => p == path,
            Event::SaveFailed { path: p, .. } => p == path,
            _ => false,
        }) {
            Event::Saved { .. } => {}
            Event::SaveFailed { reason, .. } => {
                panic!("SaveAs to {} failed: {reason}", path.display())
            }
            other => panic!("expected Saved, got {other:?}"),
        }
        self.until(
            "Rebound for the Save As",
            |ev| matches!(ev, Event::Rebound { path: p, .. } if p == path),
        );
    }

    fn close(self) {
        let Port {
            gateway,
            _state,
            _copies,
            ..
        } = self;
        assert!(gateway.close().is_ok(), "a clean quit joins the engine");
        // `_state` and `_copies` are still owned by this scope, so the temp dirs
        // outlive the join - and every assertion above.
    }
}

// ------------------------------------------------------------------- the drive

/// What one fixture did, after the full drive. `written` is the file's bytes at
/// the moment the LAST `Flush` reported `Saved`.
struct Driven {
    original: Vec<u8>,
    written: Vec<u8>,
    loaded_text: String,
    meta: FileMeta,
    asked_for_codepage: bool,
    source_still_intact: bool,
}

/// Open the copy, answer the port, arm it the way the ADR says, flush the EXACT
/// text the port handed out, and report the bytes. The invariants every case must
/// hold are asserted HERE, so no caller can forget them; what a case adds on top
/// belongs in the caller.
fn drive(entry: &Entry, codepage: u16) -> Driven {
    let source = fixtures_dir().join(&entry.file);
    let original =
        fs::read(&source).unwrap_or_else(|e| panic!("cannot read fixture {source:?}: {e}"));
    assert_eq!(
        original.len() as u64,
        entry.bytes,
        "{}: the file is {} bytes but the manifest says {} - fixture drift",
        entry.file,
        original.len(),
        entry.bytes
    );

    let mut port = Port::start(codepage);
    let copy = port.copy_of(&entry.file, &original);
    let (text, meta) = port.loaded(&copy);

    // (1) THE TRANSLATION LEG. Bytes are not the only thing the port must get
    // right: these are the facts a status line renders, and D14's code page rides
    // on the answer.
    assert_eq!(
        meta.encoding,
        encoding_from_str(&entry.encoding, codepage)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.file)),
        "{}: the port reports a different encoding than the manifest declares",
        entry.file
    );
    assert_eq!(
        meta.line_ending,
        line_ending_from_str(&entry.line_ending).unwrap_or_else(|e| panic!("{}: {e}", entry.file)),
        "{}: the port reports a different line ending than the manifest declares",
        entry.file
    );
    assert_eq!(
        meta.trailing_newline, entry.trailing_newline,
        "{}: a missing final newline is a FACT the port must carry",
        entry.file
    );
    assert_eq!(
        entry.bom_present,
        matches!(entry.encoding.as_str(), "utf8bom" | "utf16le" | "utf16be"),
        "{}: the manifest's own encoding and bom_present disagree",
        entry.file
    );
    assert!(!meta.oversize, "{}: a fixture must not trip D9", entry.file);
    assert!(
        !meta.read_only,
        "{}: a copy in a temp dir is writable",
        entry.file
    );
    // ADR-0001 option B, judged on the corpus rather than on one hand-written
    // name: the `.notes`/foreign split IS the arming rule, and it never leaks
    // from one document to the next across a 30-open sequence.
    assert_eq!(
        meta.armed, !entry.foreign,
        "{}: armed must be {} because the manifest says foreign = {}",
        entry.file, !entry.foreign, entry.foreign
    );

    // (2) READING IS NOT WRITING.
    let after_open = port.bytes_of(&copy);
    assert_eq!(
        after_open,
        original,
        "{}: the Open itself changed the file\n{}",
        entry.file,
        hex_diff(&original, &after_open)
    );

    // (3) THE FIRST FLUSH - both branches of ADR-0001, asserted on real bytes.
    match port.flushed(&copy, &text, 1) {
        Ok(revision) => {
            assert!(
                meta.armed,
                "{}: an unarmed file must not have saved",
                entry.file
            );
            assert_eq!(
                revision, 1,
                "{}: Saved reports the revision it was given",
                entry.file
            );
        }
        Err(reason) => {
            assert!(
                !meta.armed,
                "{}: an armed file skipped with {reason:?}",
                entry.file
            );
            assert_eq!(
                reason,
                SkipReason::ForeignFileNotArmed,
                "{}: the only honest refusal for a fresh foreign file",
                entry.file
            );
            let untouched = port.bytes_of(&copy);
            assert_eq!(
                untouched,
                original,
                "{}: a refused flush must write nothing\n{}",
                entry.file,
                hex_diff(&original, &untouched)
            );
            // (4) THE ONE EXPLICIT SAVE: the arming act, and a write in its own
            // right. Here the TARGET's bytes win the format decision, so this is
            // the leg that could re-encode someone's UTF-16 file as UTF-8.
            port.saved_as(&copy, &text, 2);
            let saved = port.bytes_of(&copy);
            assert_eq!(
                saved,
                original,
                "{}: the explicit save is not byte-identical\n{}",
                entry.file,
                hex_diff(&original, &saved)
            );
        }
    }

    // (5) THE AUTOSAVE LEG, unconditional: the text is exactly what
    // `Event::Loaded` gave, revision 3 is above anything the document has
    // anchored, and the epoch is the one the engine last announced.
    let saved = port.flushed(&copy, &text, 3).unwrap_or_else(|reason| {
        panic!(
            "{}: the flush on a document that must save skipped with {reason:?}",
            entry.file
        )
    });
    assert_eq!(saved, 3, "{}: Saved reports revision 3", entry.file);
    let written = port.bytes_of(&copy);
    let asked_for_codepage = port
        .host
        .calls()
        .iter()
        .any(|call| matches!(call, Call::Codepage));
    port.close();

    // (6) THE CORPUS IS READ-ONLY FROM HERE.
    let source_still_intact = fs::read(&source).is_ok_and(|again| again == original);

    Driven {
        original,
        written,
        loaded_text: text,
        meta,
        asked_for_codepage,
        source_still_intact,
    }
}

/// The shared verdict: byte-identity, and an untouched corpus.
fn verdict(entry: &Entry, driven: &Driven) {
    assert_eq!(
        driven.written,
        driven.original,
        "{}: the round trip through the PORT is not byte-identical\n{}",
        entry.file,
        hex_diff(&driven.original, &driven.written)
    );
    assert!(
        driven.source_still_intact,
        "{}: this gate wrote into the generator-owned fixture",
        entry.file
    );
}

fn find<'m>(manifest: &'m Manifest, name: &str) -> Result<&'m Entry, String> {
    manifest
        .entries
        .iter()
        .find(|e| e.file == name)
        .ok_or_else(|| format!("the manifest has no {name} fixture - the gate is incomplete"))
}

// -------------------------------------------------------------------- cases

/// THE gate for the do-no-harm rule (AGENTS.md; whitepaper 4.5) on the port leg:
/// every manifest fixture, copied into a temp dir, opened by the real engine
/// behind `Command::Open`, and flushed back through `Command::Flush` - with the
/// ADR-0001 arming save where the port refuses to write before it - must come home
/// with exactly the bytes it went out with.
#[test]
fn every_manifest_fixture_round_trips_byte_exactly_through_the_port() -> Result<(), Box<dyn Error>>
{
    let manifest = load_manifest()?;
    let mut native = 0usize;
    let mut foreign = 0usize;
    for entry in &manifest.entries {
        if entry.foreign {
            foreign += 1;
        } else {
            native += 1;
        }
        let driven = drive(entry, manifest.ansi_codepage);
        // The buffer a bridge would hold is what got flushed. Asserting it is not
        // empty is what stops a lossy `decode` from being hidden by a write that
        // happened to reproduce the same bytes.
        assert!(
            !driven.loaded_text.is_empty() || driven.original.len() <= 3,
            "{}: the port handed back no text for a file with content in it",
            entry.file
        );
        verdict(entry, &driven);
    }
    // A gate that silently narrowed to one half of the split proves nothing about
    // the other half, so both are counted out loud.
    assert!(
        native > 0 && foreign > 0,
        "the corpus must cover both halves of ADR-0001: {native} native, {foreign} foreign"
    );
    assert_eq!(
        native + foreign,
        manifest.count as usize,
        "every fixture is native or foreign"
    );
    Ok(())
}

/// Manifest and corpus must agree in BOTH directions, or the drive above can go
/// green on a shrinking list: a fixture on disk that nobody opens is exactly the
/// hole this gate exists to close.
#[test]
fn the_manifest_lists_every_fixture_on_disk_and_nothing_that_is_not() -> Result<(), Box<dyn Error>>
{
    let manifest = load_manifest()?;
    let mut listed = manifest
        .entries
        .iter()
        .map(|e| e.file.clone())
        .collect::<Vec<_>>();
    listed.sort();
    let declared = listed.clone();
    assert_eq!(
        declared.len(),
        declared
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        "the manifest lists a fixture twice, so its count cannot be trusted"
    );

    let dir = fixtures_dir();
    let mut on_disk = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read the fixture dir {dir:?}: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "manifest.json")
        .collect::<Vec<_>>();
    on_disk.sort();
    assert_eq!(
        on_disk, declared,
        "the fixture directory and the manifest disagree - regenerate the corpus, never hand-edit it"
    );
    assert_eq!(manifest.count as usize, manifest.entries.len());
    Ok(())
}

/// THE PORT-ONLY LEG - the one core's gate cannot reach: `.notes` front matter is
/// stripped on the way in and rebuilt on the way out by `api`, never by `core`.
#[test]
fn a_front_matter_note_shows_the_body_and_writes_the_header_back() -> Result<(), Box<dyn Error>> {
    let manifest = load_manifest()?;
    let entry = find(&manifest, "edge__frontmatter.notes")?;
    let driven = drive(entry, manifest.ansi_codepage);

    assert!(
        !driven.loaded_text.contains("---"),
        "the bridge must be handed the body, not the header: {:?}",
        driven.loaded_text
    );
    assert!(
        driven.written.starts_with(b"---"),
        "the file must go back WITH its front matter: {:?}",
        String::from_utf8_lossy(&driven.written)
    );
    verdict(entry, &driven);
    Ok(())
}

/// The encoding leg, on real corpus bytes rather than on a nine-character
/// literal: UTF-16 keeps its BOM and its surrogate pair, CP1252 stays
/// single-byte, and nothing is "tidied" into UTF-8.
#[test]
fn the_utf16_and_cp1252_fixtures_come_home_in_their_own_wire_form() -> Result<(), Box<dyn Error>> {
    let manifest = load_manifest()?;

    for name in [
        "edge__cjk-utf16le.txt",
        "utf16le__crlf__bom__nl.txt",
        "utf16be__crlf__bom__nl.txt",
    ] {
        let entry = find(&manifest, name)?;
        let driven = drive(entry, manifest.ansi_codepage);
        verdict(entry, &driven);
        let bom: &[u8] = if entry.encoding == "utf16le" {
            &[0xFF, 0xFE]
        } else {
            &[0xFE, 0xFF]
        };
        assert_eq!(
            &driven.written[..bom.len()],
            bom,
            "{name}: a lost BOM is a visible change to the file"
        );
    }

    for name in [
        "ansi1252__lf__nobom__nl.txt",
        "ansi1252__crlf__nobom__nonl.txt",
    ] {
        let entry = find(&manifest, name)?;
        let driven = drive(entry, manifest.ansi_codepage);
        verdict(entry, &driven);
        assert!(
            std::str::from_utf8(&driven.written).is_err(),
            "{name}: a CP1252 file that came back as valid UTF-8 was re-encoded (D14)"
        );
        assert!(
            driven.written.contains(&0xE9),
            "{name}: e-acute must go back as byte 0xE9, not as two UTF-8 bytes"
        );
        assert_eq!(
            driven.meta.encoding,
            Encoding::Ansi(GATE_CODEPAGE),
            "{name}: D14 carries the code page id on the encoding itself"
        );
        assert!(
            driven.asked_for_codepage,
            "{name}: the code page reached detect through the host seam, never a guess (D27)"
        );
    }
    Ok(())
}

/// The two shapes a helpful writer destroys: a zero-byte file (a BOM or a newline
/// added to it is a change nobody asked for) and a file that is ONLY a BOM, whose
/// entire content is the thing an encoder is tempted to strip.
#[test]
fn the_empty_and_bom_only_notes_survive_the_flush_leg() -> Result<(), Box<dyn Error>> {
    let manifest = load_manifest()?;

    let empty = find(&manifest, "edge__empty.notes")?;
    let driven = drive(empty, manifest.ansi_codepage);
    assert!(
        driven.loaded_text.is_empty(),
        "an empty file must decode to no text at all"
    );
    assert_eq!(driven.written.len(), 0, "zero bytes in, zero bytes out");
    verdict(empty, &driven);

    let bom_only = find(&manifest, "edge__bom-only.notes")?;
    let driven = drive(bom_only, manifest.ansi_codepage);
    assert!(
        driven.loaded_text.is_empty(),
        "a BOM is not content, so the buffer is empty"
    );
    assert_eq!(
        driven.written,
        vec![0xEF, 0xBB, 0xBF],
        "the file's ONLY bytes are the BOM, and they must come back"
    );
    verdict(bom_only, &driven);
    Ok(())
}

/// THE WRITE-BACK HALF, through the port: a bridge's buffer is LF-only (gpui
/// keeps LF internally, Enter inserts LF, paste CRLF→LF-replaces), so the text
/// that comes back in `Command::Flush` is NOT the bytes `Event::Loaded` was
/// built from. The format a foreign file was born with lives only in `FileMeta`
/// and in the engine's own `Detected`; if the save path never applies them, a
/// CRLF file edited once comes home as LF and the port reports `Event::Saved`
/// anyway. This is the byte-level form of that claim, on the corpus files whose
/// line endings are CRLF, judged by re-reading the disk.
///
/// Each case: copy the fixture, `Open` it, prove the refusal (`ForeignFileNotArmed`),
/// arm it with the one explicit `SaveAs` at its own path, then `Flush` an
/// LF-normalised edited buffer and hash the bytes that landed.
#[test]
fn an_edited_lf_normalised_buffer_comes_home_in_the_files_own_shape() -> Result<(), Box<dyn Error>>
{
    let manifest = load_manifest()?;
    let crlf = manifest
        .entries
        .iter()
        .filter(|e| e.line_ending == "crlf")
        .collect::<Vec<_>>();
    assert!(
        !crlf.is_empty(),
        "the corpus has no CRLF fixture, so this gate proves nothing"
    );

    let mut drift: Vec<String> = Vec::new();
    for entry in crlf {
        let source = fixtures_dir().join(&entry.file);
        let original =
            fs::read(&source).unwrap_or_else(|e| panic!("cannot read fixture {source:?}: {e}"));

        let mut port = Port::start(manifest.ansi_codepage);
        let copy = port.copy_of(&entry.file, &original);
        let (text, meta) = port.loaded(&copy);
        assert_eq!(
            meta.line_ending,
            LineEnding::CrLf,
            "{}: the port must report the file's own ending before anything is written",
            entry.file
        );

        // Arm it the way ADR-0001 says, with the buffer exactly as Loaded gave it.
        assert_eq!(
            port.flushed(&copy, &text, 1),
            Err(SkipReason::ForeignFileNotArmed),
            "{}: a fresh foreign file must refuse before the explicit save",
            entry.file
        );
        port.saved_as(&copy, &text, 2);

        // THE EDIT: what the bridge holds after one Enter/paste - LF-only line
        // breaks and a typed character on a final line the file never had.
        let edited = text.replace("\r\n", "\n").replace('\r', "\n") + "X";
        assert!(
            !edited.contains('\r'),
            "{}: the edited buffer is not LF-only, so this case tests nothing",
            entry.file
        );
        let revision = port.flushed(&copy, &edited, 3).unwrap_or_else(|reason| {
            panic!(
                "{}: the armed autosave leg skipped with {reason:?}",
                entry.file
            )
        });
        assert_eq!(
            revision, 3,
            "{}: Saved reports the flush revision",
            entry.file
        );
        let written = port.bytes_of(&copy);
        port.close();

        // Count line breaks in the WRITTEN units, not in the buffer: an escaped
        // CR is invisible to a String comparison, which is how the defect hid.
        let cr = |bytes: &[u8]| bytes.iter().filter(|&&b| b == 0x0D).count();
        let lf = |bytes: &[u8]| bytes.iter().filter(|&&b| b == 0x0A).count();
        // Every file in this half of the corpus is UTF-8/UTF-16 with CRLF pairs;
        // UTF-16 carries each CR and LF in one unit, so the CR count is the pair
        // count either way, and a lost CR shows up as cr < lf.
        if cr(&written) != lf(&written) {
            drift.push(format!(
                "{}: an LF-normalised edit wrote {} CR bytes against {} LF bytes - the file's CRLF pairs were not restored",
                entry.file,
                cr(&written),
                lf(&written)
            ));
        }
        // The final-newline fact, read back out of the bytes: a `__nonl__`
        // fixture must not have gained one and a `__nl__` fixture must not have
        // lost it. UTF-16 spells the LF as 0A 00, hence both spellings.
        let ends_with_newline = written.ends_with(&[0x0A]) || written.ends_with(&[0x0A, 0x00]);
        if ends_with_newline != entry.trailing_newline {
            drift.push(format!(
                "{}: the saved file ends with a final newline = {ends_with_newline}, the file's own fact is {}",
                entry.file, entry.trailing_newline
            ));
        }
        // Both shape assertions above are satisfiable by writing the fixture's
        // OWN bytes back and saving nothing, so the edit has to be visible too.
        assert!(
            written != original,
            "{}: the flush wrote the fixture's bytes back unchanged - the edit never reached the disk",
            entry.file
        );
        assert!(
            fs::read(&source).is_ok_and(|again| again == original),
            "{}: this gate wrote into the generator-owned fixture",
            entry.file
        );
    }

    assert!(
        drift.is_empty(),
        "{} CRLF fixture(s) lost their shape to an edited buffer through the port:\n{}",
        drift.len(),
        drift.join("\n")
    );
    Ok(())
}

/// The EOL leg, on the corpus files whose newlines are not uniform - plus the
/// CJK/emoji UTF-8 file, the IME commit string's own bytes. The two `.notes`
/// among them are armed on open, so their FIRST flush is the write.
#[test]
fn lone_cr_mixed_eol_and_cjk_keep_every_byte_through_the_port() -> Result<(), Box<dyn Error>> {
    let manifest = load_manifest()?;
    for name in [
        "edge__lone-cr.notes",
        "edge__mixed-eol.notes",
        "edge__cjk-utf8.txt",
    ] {
        let entry = find(&manifest, name)?;
        let driven = drive(entry, manifest.ansi_codepage);
        if !entry.foreign {
            assert!(
                driven.meta.armed,
                "{name}: a .notes file arms on open, so its first Flush is the write"
            );
        }
        let cr = |bytes: &[u8]| bytes.iter().filter(|&&b| b == 0x0D).count();
        assert_eq!(
            cr(&driven.written),
            cr(&driven.original),
            "{name}: a dropped CR is invisible to a String comparison"
        );
        verdict(entry, &driven);
    }
    Ok(())
}
