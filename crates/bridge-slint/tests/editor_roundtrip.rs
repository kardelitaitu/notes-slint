//! THE BRIDGE LEG of the do-no-harm rule (whitepaper 4.5, AGENTS.md "Do no harm"), proven on
//! the bridge this repo actually ships - NOT copied from crates/bridge-gpui/tests/
//! editor_roundtrip.rs. ADR-0006 item 2: bridge-gpui is frozen, and its proofs have to be
//! re-earned on Slint in Slint's own terms before its name means anything as evidence.
//!
//! WHY A COPY WOULD HAVE BEEN THE WRONG PROOF. The gpui twin proves a glass jar:
//! Editor::with_content stores what it is given verbatim, carriage returns and all, and its own
//! header says that normalising on the way in would be the bug ("a buffer that normalised would
//! be harmless for a newline and fatal for a BOM"). THIS bridge is not a glass jar, and the
//! difference is in the source, not in the prose - Event::Loaded is handled at
//! crates/bridge-slint/src/surface.rs, `drain`'s `Event::Loaded` arm, as "let adopted =
//! lf(text); p.last_sent = adopted.clone();", where lf is crates/bridge-slint/src/plumbing.rs:49-51 replacing every CRLF
//! and then every lone CR with LF, and that adopted string is what goes INTO the widget
//! (surface.rs, the same `Event::Loaded` arm, ui.set_buffer(adopted)). Two facts forced that shape, and they are separate:
//! a Slint TextEdit addresses its document by line and its line model cuts on LF, so a lone CR
//! has nowhere to live in it; and the pump that decides whether to send reads the widget back
//! through the SAME lf() (surface.rs, `text_pump`'s identical-check), so adoption verbatim plus comparison normalised would
//! make every CRLF file look edited the instant it opened. The jar is deliberately not glass, and
//! a twin that demanded glass would assert a contract this bridge does not hold.
//!
//! SO THE TWIN PROVES THE THREE THINGS THAT ARE TRUE HERE, one test each:
//!   (a) SUPPRESSION - an untouched foreign file adopts to last_sent and therefore produces NO
//!       Flush, and the file on disk is left byte-identical by the open and by the decision
//!       (an_untouched_foreign_file_adopts_to_last_sent_and_wakes_no_flush). Slint needs this leg
//!       more than gpui does: gpui's jar could not lie. This one can.
//!   (b) DOMINANT-EOL RESTORATION - the normalisation is paid back at the save layer. Core keeps
//!       LF internally and re-emits the file's own ending in layout()
//!       (crates/core/src/encoding.rs, `layout`, whose LineEnding::CrLf arm folds every kind of
//!       break back into CRLF), so an edited buffer still writes CRLF bytes
//!       (an_edited_crlf_file_goes_back_as_crlf_through_the_ports_save_layer). Both halves of
//!       "bytes-out == bytes-in" are asserted: the arming save of an UNTOUCHED buffer reproduces
//!       the fixture exactly, and a real keystroke reproduces every CR it had and adds one.
//!   (c) THE DIVERGENCE, NAMED RATHER THAN SKIPPED - the next block states it, and the test that
//!       names it measures it, so the claim is bytes and not a rumour
//!       (the_named_divergence_a_lone_cr_or_mixed_eol_file_cannot_come_back_unchanged).
//!
//! THE DIVERGENCE, STATED AS A DOCUMENTED-KNOWN LIMIT AND NOT SILENTLY SKIPPED: after any edit, a
//! lone-CR file and a mixed-EOL file CANNOT return byte-identical through a Slint TextEdit, because
//! the widget was never given the bytes to return. crates/core/tests/fixtures/edge__lone-cr.notes
//! ("alpha CR beta CR gamma") and crates/core/tests/fixtures/edge__mixed-eol.notes
//! ("alpha LF beta CRLF gamma CR") are the two corpus rows; core's own gate saves them byte-exact,
//! this bridge's cannot. The fix is not a line-ending rule - layout()'s LineEnding::Lf arm is
//! already verbatim, and the loss happens upstream of it, in the adoption. It is ORIGINAL-TEXT
//! PRESERVATION: hold the bytes the port handed out, send them while the buffer still equals them,
//! and keep each CR's position to rebuild it after an edit - i.e. new state and a new comparison
//! inside surface.rs's Pump. That edit is not this file's fence, so it is NOT built here. It is not
//! a TODO either: the loss is asserted in the third test, and the day it is fixed that test fails
//! on purpose and gets edited on purpose. Two other shapes do survive the jar, which is why this is
//! a named limit and not a general admission: a BOM is stripped by core into bom_present and
//! re-prepended on encode, so no U+FEFF rides in the text the widget is given, and the LF-dominant
//! rows are already what lf() returns, so they round-trip untouched.
//!
//! WHY THERE IS NO "#[path = ../src/...] mod ...;" IN THIS FILE, which is the one structural
//! difference from every other bridge test here. bridge-slint has no lib target, so a path include
//! is the only way in - and plumbing.rs cannot be included on its own: it opens with
//! "use crate::Spike;", "use crate::surface::Pump;" and "use crate::title_contract;", so the whole
//! crate graph comes with it, surface.rs and product.rs included, and those are the bytes the
//! concurrent drag lane is editing under me. A test whose green depends on a file someone else is
//! mid-edit on is not a proof. So the two-line normaliser is COPIED below, with the cite - and the
//! copy is not taken on trust: the_test_copied_normaliser_still_matches_the_bridge reads the
//! shipped source with include_str! and fails if either the function body or the adoption site
//! drifts. THE TRADEOFF, STATED: a twin that re-implements the function proves the CONTRACT (lf is
//! total, pure, and the only writer of the rule), not the call sites; the call sites stay with
//! surface.rs's own needles. Naming both halves is the honest shape - and the contract proved here
//! is the one this bridge holds, which is the whole point of writing the twin again.
//!
//! THE ARRANGEMENT. The drive is the api Gateway, headless, the way crates/api/tests/
//! roundtrip_port.rs drives it - Command::Open, the Event::Loaded answer, Command::SaveAs as
//! ADR-0001's arming act, Command::Flush, Event::Saved - because that is the only save path a
//! bridge is allowed to reach (AGENTS.md: a bridge imports api and its own toolkit, nothing else),
//! which is also why layout() is proved through the bytes on disk rather than by calling it. The
//! text sent on every leg is the string THIS bridge's jar holds after adoption, never the
//! fixture's raw decoded text - that substitution is the file. The corpus is read-only: each case
//! opens a COPY in a temp dir, and crates/core/tests/fixtures is re-read at the end to prove
//! nothing was written into it. No src/ edit and no new dependency: the temp dirs are std::fs, so
//! tempfile stays out of this crate's manifest.
//!
//! The #![allow(unused_crate_dependencies)] below is a property of the arrangement, not of what is
//! asserted: a test target is its own crate, and this one uses notes_api and slint while
//! raw-window-handle and rfd belong to product.rs and probe.rs alone.

// See the docs above: the lint is package-wide, and a test crate that names two of its four
// dependencies is telling the truth about this file, not hiding a problem.
#![allow(unused_crate_dependencies)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notes_api::{Command, Encoding, Event, FileMeta, Gateway, LineEnding, Settings, StateDir};

// ---------------------------------------------------------------- the bridge's own rule

/// The shipped normaliser, crates/bridge-slint/src/plumbing.rs:49-51, copied rather than included
///     the reason is in the header ("WHY THERE IS NO path include"), and the copy is checked
/// against the shipped file by the_test_copied_normaliser_still_matches_the_bridge.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The exact body of the shipped lf, as bytes in the shipped source. A literal of a literal: the
/// backslashes below are CHARACTERS in plumbing.rs, which is why they are doubled here.
const SHIPPED_LF_BODY: &str = "text.replace(\"\\r\\n\", \"\\n\").replace('\\r', \"\\n\")";

/// What surface.rs's `drain` / `Event::Loaded` arm does with a Loaded text, named so the
/// tests read as the contract.
fn adopt(text: &str) -> String {
    lf(text)
}

/// text_pump's guard (surface.rs, `text_pump`'s identical-check, and the same comparison in
/// its `text_pump_by_compare` fallback): the string read out of the widget goes through lf() and is compared with
/// last_sent. Equal means nothing is sent.
fn flush_due(buffer: &str, last_sent: &str) -> bool {
    lf(buffer) != last_sent
}

/// Every line break in bytes is a CRLF pair: no bare LF, no bare CR. The shape a CRLF file must
/// come back in, stated on bytes rather than on a String a lossy decode could hide.
fn crlf_only(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if bytes.get(i + 1) != Some(&b'\n') {
                    return false;
                }
                i += 2;
            }
            b'\n' => return false,
            _ => i += 1,
        }
    }
    true
}

fn count(hay: &[u8], needle: u8) -> usize {
    hay.iter().filter(|&&b| b == needle).count()
}

/// The corpus, by path - the same directory crates/api/tests/roundtrip_port.rs judges, read the
/// same read-only way. CARGO_MANIFEST_DIR is crates/bridge-slint, so up one.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/bridge-slint has a parent")
        .join("core")
        .join("tests")
        .join("fixtures")
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {path:?}: {e}"))
}

fn read_back(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("cannot re-read {path:?}: {e}"))
}

/// The rows this file drives, chosen for the shapes the header names.
const CRLF_WITH_TRAILING_NEWLINE: &str = "utf8__crlf__nobom__nl.txt";
const LOSSY_SHAPES: [&str; 2] = ["edge__lone-cr.notes", "edge__mixed-eol.notes"];

// ------------------------------------------------------------------- the temp dirs

/// Three directories, all private to one case: the root, the engine's StateDir inside it (so no
/// case can see another's session.json), and a SEPARATE one for the COPY of the fixture, because a
/// note this app did not create lives nowhere near its state - and the corpus must never be the
/// write target. tempfile is not a dependency of this crate and adding one is a manifest edit, so
/// these are std::fs, removed on drop.
struct Sandbox {
    root: PathBuf,
    state: PathBuf,
    notes: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut root = std::env::temp_dir();
        root.push(format!(
            "notes-bridge-slint-roundtrip-{tag}-{}-{stamp}",
            std::process::id()
        ));
        let state = root.join("state");
        let notes = root.join("notes");
        for dir in [&state, &notes] {
            fs::create_dir_all(dir).unwrap_or_else(|e| panic!("cannot create {dir:?}: {e}"));
        }
        Sandbox { root, state, notes }
    }

    /// Lays a fixture's bytes down under its OWN name - the extension is part of the case, since
    /// the .notes ending is what decides both arming and front matter.
    fn place(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.notes.join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("cannot lay down {path:?}: {e}"));
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // The worst this can say is "the OS will not delete a temp dir", and a failing test over
        // leaked scratch is worse than the leak.
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------- the drive

/// Long enough that a loaded machine cannot flake, short enough that a hung engine fails the run
/// instead of hanging it. The same constant and the same reasoning as roundtrip_port.rs:72.
const ANSWER: Duration = Duration::from_secs(5);

/// One running port, its event wire, and the generation the engine last announced - echoed on
/// every Flush the way surface.rs's pump stores it from Loaded/Rebound (:947), never invented.
struct Port {
    gateway: Gateway,
    rx: Receiver<Event>,
    epoch: u64,
}

impl Port {
    /// The real port, with the host the port builds for itself (D46). No window is ever registered
    /// and none is needed: a handle is what geometry and pinning ask for, and this file asks only
    /// about bytes. Gateway::start is what product.rs calls; reaching for start_with_host instead
    /// would need a notes-platform type, which a bridge may not import, so this is the honest door.
    fn start(state_dir: &Path) -> Self {
        let (gateway, rx) = Gateway::start(StateDir(state_dir.to_path_buf()), Settings::default());
        Port {
            gateway,
            rx,
            epoch: 0,
        }
    }

    fn send(&self, command: Command) {
        assert!(
            self.gateway.send(command.clone()).is_ok(),
            "the engine must still be accepting commands: {command:?}"
        );
    }

    /// THE WIRE: every Loaded/Rebound that flows past updates the echoed generation, so a Flush can
    /// never be one bump out of step.
    fn until<F>(&mut self, want: &str, f: F) -> Event
    where
        F: Fn(&Event) -> bool,
    {
        let mut seen = Vec::new();
        let deadline = Instant::now() + ANSWER;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(event) => {
                    if let Event::Loaded { epoch, .. } | Event::Rebound { epoch, .. } = &event {
                        self.epoch = *epoch;
                    }
                    if f(&event) {
                        return event;
                    }
                    seen.push(format!("{event:?}"));
                }
                Err(err) => panic!("never saw {want}; saw {seen:?} first ({err:?})"),
            }
        }
    }

    /// The act the bridge performs on Open / ctrl-o, answered: the text and the facts.
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

    /// A Flush, answered honestly: either it saved, or the engine said WHY nothing happened
    /// (ADR-0001 and D11 - silence is forbidden). A SaveFailed here panics: a refused write is not
    /// a skip, and every case here opens a writable copy.
    fn flushed(&mut self, path: &Path, text: &str, revision: u64) {
        self.send(Command::Flush {
            text: text.to_owned(),
            revision,
            epoch: self.epoch,
        });
        match self.until("a verdict on the Flush", |ev| {
            matches!(
                ev,
                Event::Saved { .. } | Event::AutosaveSkipped { .. } | Event::SaveFailed { .. }
            )
        }) {
            Event::Saved { path: at, .. } => {
                assert_eq!(&at, path, "Saved named a file other than the open one");
            }
            Event::AutosaveSkipped { reason } => {
                panic!("the flush to {} was skipped: {reason:?}", path.display());
            }
            Event::SaveFailed { reason, .. } => {
                panic!("the flush to {} failed: {reason}", path.display());
            }
            other => panic!("expected Saved, got {other:?}"),
        }
    }

    /// The one explicit act ADR-0001 requires of a foreign file, at the file's OWN path - which is
    /// what the live bridge puts behind ctrl-s. Saved then Rebound, and the Rebound carries the new
    /// generation.
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

    /// THE SUPPRESSION WITNESS, on the wire rather than on a variable: nothing was sent, so the
    /// port must produce no verdict on the open document at all. A save event here is the exact
    /// failure this leg exists to catch - text nobody typed reaching the disk.
    fn expect_no_save(&mut self, grace: Duration) {
        let deadline = Instant::now() + grace;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match self.rx.recv_timeout(left) {
                Ok(event) => assert!(
                    !matches!(
                        event,
                        Event::Saved { .. }
                            | Event::SaveFailed { .. }
                            | Event::AutosaveSkipped { .. }
                    ),
                    "an untouched document woke the save layer: {event:?}"
                ),
                Err(_) => return,
            }
        }
    }

    fn close(self) {
        assert!(
            self.gateway.close().is_ok(),
            "a clean quit joins the engine"
        );
    }
}

/// A failure message that can be read: what the save actually produced, summarised by shape.
fn byte_shape(bytes: &[u8]) -> String {
    format!(
        "{} bytes, {} CRLF pairs, {} LF, {} CR",
        bytes.len(),
        bytes.windows(2).filter(|w| *w == b"\r\n").count(),
        count(bytes, b'\n'),
        count(bytes, b'\r')
    )
}

// --------------------------------------------------------------------- the honesty leg

/// The copy above is the shipped rule or it is nothing, so the shipped source is read and compared.
/// This is also the guard the gpui twin has no need for: the day plumbing.rs folds something else
/// into the adoption, or surface.rs stops adopting through it at all, this bridge's contract has
/// changed and this file says so before its other three tests quietly start proving an old world.
#[test]
fn the_test_copied_normaliser_still_matches_the_bridge() {
    let shipped = include_str!("../src/plumbing.rs");
    assert!(
        shipped.contains(SHIPPED_LF_BODY),
        "crates/bridge-slint/src/plumbing.rs no longer contains {SHIPPED_LF_BODY:?} - the lf this \
         test re-implements has drifted, so re-read it, re-state the header's contract, and edit \
         this literal on purpose"
    );
    let surface = include_str!("../src/surface.rs");
    assert!(
        surface.contains("let adopted = lf(text);"),
        "surface.rs no longer adopts a Loaded text through lf() - either the divergence this file \
         names has been fixed, or the adoption moved; the header needs rewriting either way"
    );
}

// ----------------------------------------------------------------------- the three legs

/// (a) SUPPRESSION. An untouched foreign file produces no Flush, because last_sent IS what the
/// widget was given - proved twice, once on the variable that guards the send and once on the
/// bytes nobody should have touched.
#[test]
fn an_untouched_foreign_file_adopts_to_last_sent_and_wakes_no_flush() {
    let sbox = Sandbox::new("suppress");
    let original = read_fixture(CRLF_WITH_TRAILING_NEWLINE);
    let copy = sbox.place(CRLF_WITH_TRAILING_NEWLINE, &original);

    let mut port = Port::start(&sbox.state);
    let (text, meta) = port.loaded(&copy);
    assert_eq!(
        meta.line_ending,
        LineEnding::CrLf,
        "the port must still report a CRLF file as CRLF, or there is nothing to preserve"
    );
    assert!(
        meta.trailing_newline,
        "the fixture ends with a newline, and the port's own fact must say so"
    );
    assert!(
        !meta.armed,
        "a foreign .txt copy is disarmed by ADR-0001, so a spurious Flush would at least be \
         refused - and would still light the dot and lie about the buffer"
    );
    assert!(!meta.read_only, "a copy in a temp dir is writable");

    // THE ADOPTION, as surface.rs's `drain` / `Event::Loaded` arm performs it.
    let last_sent = adopt(&text);
    assert!(
        !last_sent.contains('\r'),
        "the jar holds CR-free text, which is what makes this bridge not a glass jar: {last_sent:?}"
    );
    assert_ne!(
        last_sent, text,
        "the file really does carry carriage returns - if it did not, the guard below would be \
         comparing two equal strings by accident and proving nothing"
    );

    // The widget holds exactly the adopted string (surface.rs, the same arm's
    // `set_buffer`), and SharedString is byte-exact
    // about it: the lossy part of this bridge is the ADOPTION, not the storage, and a claim made
    // against the toolkit's own type is worth more than one made against a String stand-in.
    let buffer: slint::SharedString = last_sent.clone().into();
    assert_eq!(
        buffer.as_str(),
        last_sent,
        "Slint's own string type must not be the thing that edits the bytes"
    );
    // THE GUARD: text_pump reads the widget back through lf() and compares. Equal means no
    // Command::Flush, so no revision moves and no autosave wakes.
    assert!(
        !flush_due(buffer.as_str(), &last_sent),
        "an untouched file must produce NO Flush - the pump would send text nobody typed"
    );

    // THE COUNTER-FACTUAL that keeps the assertion above from being vacuous. The gpui bridge can
    // adopt this text verbatim and stay silent, because its pump compares verbatim bytes too.
    // Adopt verbatim HERE, against the comparison surface.rs actually runs, and an untouched CRLF
    // file looks edited the moment it opens - which is why the divergence in the header is a
    // contract and not an oversight.
    assert!(
        flush_due(&text, &text),
        "a verbatim adoption in a pump that compares through lf() would have flushed a file \
         nobody edited - this is the harm the Slint jar exists to avoid"
    );

    // NOTHING REACHED THE PORT, and so nothing reached the disk: the open is the only command this
    // case ever sent.
    port.expect_no_save(Duration::from_millis(250));
    assert_eq!(
        read_back(&copy),
        original,
        "reading a foreign file through this bridge changed its bytes"
    );
    assert_eq!(
        read_fixture(CRLF_WITH_TRAILING_NEWLINE),
        original,
        "the corpus itself was written to - crates/core/tests/fixtures is read-only from here"
    );
    port.close();
}

/// (b) DOMINANT-EOL RESTORATION. The bridge may normalise on the way in because core puts the
/// file's own ending back on the way out - asserted on the arming save of an untouched buffer
/// (bytes-out == bytes-in, every CR where it was) and again after a real keystroke.
#[test]
fn an_edited_crlf_file_goes_back_as_crlf_through_the_ports_save_layer() {
    let sbox = Sandbox::new("crlf");
    let original = read_fixture(CRLF_WITH_TRAILING_NEWLINE);
    let copy = sbox.place(CRLF_WITH_TRAILING_NEWLINE, &original);
    assert!(
        crlf_only(&original) && count(&original, b'\r') > 0,
        "the fixture must still be a CRLF file, or this case proves an empty claim"
    );

    let mut port = Port::start(&sbox.state);
    let (text, meta) = port.loaded(&copy);
    assert_eq!(
        meta.encoding,
        Encoding::Utf8,
        "this leg is about endings, so the encoding must be the one with no code page in it"
    );
    let last_sent = adopt(&text);

    // THE ARMING ACT: ctrl-s on a foreign file, which is this bridge's SaveAs at the file's own
    // path - and the text it carries is the buffer AS THE BRIDGE HOLDS IT, LF-only, because that is
    // the only text a Slint TextEdit can hand back.
    port.saved_as(&copy, &last_sent, 1);
    let armed = read_back(&copy);
    assert_eq!(
        armed,
        original,
        "an LF-only buffer saved back a CRLF file and did not reproduce it byte for byte - core's \
         layout() re-emits the file's own ending, and what came out was {}",
        byte_shape(&armed)
    );

    // THE EDIT. Typing at the end of a terminated file leaves an unterminated last line, which is
    // exactly the buffer a user produces and the case layout()'s trailing-newline rule exists for:
    // the file ended with a newline, so the saved one must too, in the file's own ending.
    let edited = format!("{last_sent}X");
    assert!(
        !edited.contains('\r'),
        "the edit came from the LF side of the seam, so no CR in it can be the bridge's doing"
    );
    port.flushed(&copy, &edited, 2);

    let after = read_back(&copy);
    let want = [original.as_slice(), b"X\r\n"].concat();
    assert_eq!(
        after,
        want,
        "one keystroke rewrote the file's line endings or lost its trailing newline: {}",
        byte_shape(&after)
    );
    assert!(
        crlf_only(&after),
        "the saved file must not mix endings: {}",
        byte_shape(&after)
    );
    assert_eq!(
        count(&after, b'\r'),
        count(&original, b'\r') + 1,
        "the file kept its own carriage returns and gained exactly the one the new line needed"
    );
    assert_ne!(after, original, "the edit never reached the disk");
    assert_eq!(
        read_fixture(CRLF_WITH_TRAILING_NEWLINE),
        original,
        "the corpus itself was written to"
    );
    port.close();
}

/// (c) THE DIVERGENCE, MEASURED. The header says why it is a limit of the bridge and not of the
/// port, and why fixing it needs original-text preservation rather than a line-ending rule. This
/// case does not excuse the loss; it puts the loss on the record in bytes, so "documented-known" is
/// a measurement and not an apology - and the fix, when it comes, is announced by this test
/// failing.
#[test]
fn the_named_divergence_a_lone_cr_or_mixed_eol_file_cannot_come_back_unchanged() {
    for name in LOSSY_SHAPES {
        let sbox = Sandbox::new("divergence");
        let original = read_fixture(name);
        let raw = std::str::from_utf8(&original)
            .unwrap_or_else(|e| panic!("fixture {name} must be UTF-8 for this leg: {e}"));
        let copy = sbox.place(name, &original);
        assert!(
            count(&original, b'\r') > 0,
            "{name}: the row this leg exists for has no carriage returns left in it"
        );

        let mut port = Port::start(&sbox.state);
        let (text, meta) = port.loaded(&copy);
        assert_eq!(
            text, raw,
            "{name}: the port hands out the file's own characters, CR included - so the loss is \
             downstream of it, in the adoption"
        );
        assert_eq!(
            meta.line_ending,
            LineEnding::Lf,
            "{name}: core calls a lone-CR or mixed file Lf, and layout()'s Lf arm is verbatim, so \
             nothing on the save side can put a CR back"
        );
        assert!(
            meta.armed,
            "{name}: a .notes fixture is armed, so the flush below really writes - the loss is on \
             disk, not hypothetical"
        );

        // No keystroke is even needed to state the loss, but the bridge's own flush is what carries
        // it, so send exactly what the pump would send after a load.
        let last_sent = adopt(&text);
        assert!(
            !last_sent.contains('\r'),
            "{name}: every carriage return is gone before the widget is even filled"
        );
        assert_ne!(
            last_sent, text,
            "{name}: the adoption is lossy for this shape - if this fires the other way the bridge \
             stopped normalising and this file's header is out of date"
        );
        port.flushed(&copy, &last_sent, 1);

        let after = read_back(&copy);
        assert_eq!(
            after,
            last_sent.as_bytes(),
            "{name}: what came back is the normalised text verbatim, which IS the divergence"
        );
        assert_ne!(
            after, original,
            "{name}: a lone-CR or mixed-EOL file came back byte-identical, so the documented-known \
             limit this file names no longer holds - update the header with this test"
        );
        assert_eq!(
            count(&after, b'\r'),
            0,
            "{name}: {} carriage returns went in and the saved file shows the loss exactly, not \
             part of it",
            count(&original, b'\r')
        );
        assert_eq!(
            read_fixture(name),
            original,
            "the corpus itself was written to"
        );
        port.close();
    }
}
