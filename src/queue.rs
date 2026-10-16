//! The file where Android and Linux keep events until a UI handler takes them.
//!
//! The file starts with [`HEADER`] and holds records appended one by one. A record is a tag
//! byte followed by its fields, and lengths are little-endian `u32`.
//!
//! | Tag | Entry | Fields |
//! |---|---|---|
//! | 1 | message | `started_app` (0 or 1), payload length, payload |
//! | 2 | FCM token | length, UTF-8 token |
//! | 3 | messages dropped | none |
//! | 4 | Web Push token | endpoint length, UTF-8 endpoint, 65-byte `p256dh`, 16-byte `auth` |
//! | 5 | registration failed | length, UTF-8 reason |
//!
//! One process owns the file at a time, holding an exclusive lock on [`QueueFile::lock_path`]
//! from its first operation until it exits. A process killed mid-append leaves a short last
//! record, which reading ignores and the next owner cuts off when it takes the file over, so the
//! records after it stay readable.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::{Event, Message, Token};

/// How long a process waits for the queue's previous owner, such as a headless process that is
/// finishing its calls before it exits.
const CLAIM_BOUND: Duration = Duration::from_secs(30);
/// How often a waiting process tries the lock again.
const CLAIM_POLL: Duration = Duration::from_millis(50);

/// Names the format and its version.
pub(crate) const HEADER: &[u8; 8] = b"PUSHUPS\x01";

const MESSAGE: u8 = 1;
const FCM_TOKEN: u8 = 2;
const MESSAGES_DROPPED: u8 = 3;
const WEB_PUSH_TOKEN: u8 = 4;
const REGISTRATION_FAILED: u8 = 5;

/// An event waiting in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Message(Message),
    FcmToken(String),
    MessagesDropped,
    WebPushToken {
        endpoint: String,
        p256dh: [u8; 65],
        auth: [u8; 16],
    },
    RegistrationFailed(String),
}

impl Entry {
    /// The distributor dropped the app's registration, on Android or Linux.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub(crate) fn unregistered() -> Self {
        Self::RegistrationFailed("the distributor unregistered the app".to_owned())
    }

    pub(crate) fn into_event(self) -> Event {
        match self {
            Self::Message(message) => Event::Message(message),
            Self::FcmToken(token) => Event::Token(Token::Fcm(token)),
            Self::MessagesDropped => Event::MessagesDropped,
            Self::WebPushToken {
                endpoint,
                p256dh,
                auth,
            } => Event::Token(Token::WebPush {
                endpoint,
                p256dh,
                auth,
                expires: None,
            }),
            Self::RegistrationFailed(reason) => Event::registration_failed(reason),
        }
    }

    /// Appends the record of this entry to `out`.
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Message(message) => {
                out.push(MESSAGE);
                out.push(u8::from(message.started_app));
                push_bytes(out, &message.payload)
            }
            Self::FcmToken(token) => {
                out.push(FCM_TOKEN);
                push_bytes(out, token.as_bytes())
            }
            Self::MessagesDropped => {
                out.push(MESSAGES_DROPPED);
                Ok(())
            }
            Self::WebPushToken {
                endpoint,
                p256dh,
                auth,
            } => {
                out.push(WEB_PUSH_TOKEN);
                push_bytes(out, endpoint.as_bytes())?;
                out.extend_from_slice(p256dh);
                out.extend_from_slice(auth);
                Ok(())
            }
            Self::RegistrationFailed(reason) => {
                out.push(REGISTRATION_FAILED);
                push_bytes(out, reason.as_bytes())
            }
        }
    }
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry over 4 GiB"))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Reads the entries of a queue file, up to the first record that is short or unknown.
pub(crate) fn decode(bytes: &[u8]) -> Vec<Entry> {
    let Some(mut rest) = bytes.strip_prefix(HEADER.as_slice()) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    while let Some((record, after)) = Record::parse(rest) {
        entries.push(record.to_entry());
        rest = after;
    }
    entries
}

/// The length of the header and the complete records after it, or `None` when the bytes do not
/// start with the header. Nothing is copied.
fn readable_len(bytes: &[u8]) -> Option<usize> {
    let mut rest = bytes.strip_prefix(HEADER.as_slice())?;
    while let Some((_, after)) = Record::parse(rest) {
        rest = after;
    }
    Some(bytes.len() - rest.len())
}

/// One record as it lies in the file, borrowed from its bytes.
enum Record<'bytes> {
    Message {
        started_app: bool,
        payload: &'bytes [u8],
    },
    FcmToken(&'bytes str),
    MessagesDropped,
    WebPushToken {
        endpoint: &'bytes str,
        p256dh: &'bytes [u8; 65],
        auth: &'bytes [u8; 16],
    },
    RegistrationFailed(&'bytes str),
}

impl<'bytes> Record<'bytes> {
    /// The record at the start of `bytes` and the bytes after it, or `None` when it is short or
    /// unknown.
    fn parse(bytes: &'bytes [u8]) -> Option<(Self, &'bytes [u8])> {
        let (&tag, rest) = bytes.split_first()?;
        match tag {
            MESSAGE => {
                let (&started_app, rest) = rest.split_first()?;
                let started_app = match started_app {
                    0 => false,
                    1 => true,
                    _ => return None,
                };
                let (payload, rest) = take_bytes(rest)?;
                Some((
                    Self::Message {
                        started_app,
                        payload,
                    },
                    rest,
                ))
            }
            FCM_TOKEN => {
                let (token, rest) = take_text(rest)?;
                Some((Self::FcmToken(token), rest))
            }
            MESSAGES_DROPPED => Some((Self::MessagesDropped, rest)),
            WEB_PUSH_TOKEN => {
                let (endpoint, rest) = take_text(rest)?;
                let (p256dh, rest) = rest.split_first_chunk::<65>()?;
                let (auth, rest) = rest.split_first_chunk::<16>()?;
                Some((
                    Self::WebPushToken {
                        endpoint,
                        p256dh,
                        auth,
                    },
                    rest,
                ))
            }
            REGISTRATION_FAILED => {
                let (reason, rest) = take_text(rest)?;
                Some((Self::RegistrationFailed(reason), rest))
            }
            _ => None,
        }
    }

    fn to_entry(&self) -> Entry {
        match *self {
            Self::Message {
                started_app,
                payload,
            } => Entry::Message(Message {
                payload: payload.to_vec(),
                started_app,
            }),
            Self::FcmToken(token) => Entry::FcmToken(token.to_owned()),
            Self::MessagesDropped => Entry::MessagesDropped,
            Self::WebPushToken {
                endpoint,
                p256dh,
                auth,
            } => Entry::WebPushToken {
                endpoint: endpoint.to_owned(),
                p256dh: *p256dh,
                auth: *auth,
            },
            Self::RegistrationFailed(reason) => Entry::RegistrationFailed(reason.to_owned()),
        }
    }
}

fn take_bytes(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = bytes.split_first_chunk::<4>()?;
    let length = usize::try_from(u32::from_le_bytes(*length)).ok()?;
    if rest.len() < length {
        return None;
    }
    Some(rest.split_at(length))
}

fn take_text(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let (text, rest) = take_bytes(bytes)?;
    Some((std::str::from_utf8(text).ok()?, rest))
}

/// The queue file at a fixed path.
#[derive(Debug)]
pub(crate) struct QueueFile {
    path: PathBuf,
    /// The lock file, once this process owns the queue. The lock lasts as long as the file is
    /// open, which the kernel ends when the process exits however it exits.
    owner: OnceLock<File>,
}

impl QueueFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            owner: OnceLock::new(),
        }
    }

    /// The lock file beside the queue.
    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    /// Makes this process the queue's owner, waiting up to 30 s for a previous owner to exit. The
    /// owner first cuts off the bytes no reader accepts, a header or a last record a killed
    /// process cut short, so its appends follow the readable records. A second call does nothing.
    pub(crate) fn claim(&self) -> io::Result<()> {
        self.claim_within(CLAIM_BOUND)
    }

    fn claim_within(&self, bound: Duration) -> io::Result<()> {
        if self.owner.get().is_some() {
            return Ok(());
        }
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.lock_path())?;
        let deadline = Instant::now() + bound;
        while !try_lock(&lock)? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "another process still holds the queue at {}",
                        self.path.display()
                    ),
                ));
            }
            std::thread::sleep(CLAIM_POLL);
        }
        self.cut_unreadable()?;
        // The caller's lock keeps a second claim out, and a lost race would only drop a second
        // handle on the same lock.
        let _ = self.owner.set(lock);
        Ok(())
    }

    fn cut_unreadable(&self) -> io::Result<()> {
        let mut file = match OpenOptions::new().read(true).write(true).open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        // A first append cut inside the header leaves a prefix no reader accepts.
        let readable = readable_len(&bytes).unwrap_or(0);
        if readable < bytes.len() {
            file.set_len(u64::try_from(readable).map_err(io::Error::other)?)?;
            file.sync_data()?;
        }
        Ok(())
    }

    /// Appends `entry` and waits until it is on storage. A write that fails part way is cut back
    /// off, so the next record still follows a readable one. The caller's lock keeps this
    /// process's other appends and drains out meanwhile.
    pub(crate) fn append(&self, entry: &Entry) -> io::Result<()> {
        self.claim()?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.path)?;
        let before = file.seek(SeekFrom::End(0))?;
        let mut record = Vec::new();
        if before == 0 {
            record.extend_from_slice(HEADER);
        }
        entry.encode(&mut record)?;
        let written = file.write_all(&record).and_then(|()| file.sync_data());
        if written.is_err() {
            let _ = file.set_len(before);
        }
        written
    }

    /// Returns every entry in append order and empties the file.
    pub(crate) fn take_all(&self) -> io::Result<Vec<Entry>> {
        self.claim()?;
        let mut file = match OpenOptions::new().read(true).write(true).open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        empty(&file)?;
        Ok(decode(&bytes))
    }
}

fn empty(file: &File) -> io::Result<()> {
    file.set_len(0)?;
    file.sync_data()
}

/// Takes the exclusive lock on `lock` if no other open file holds it, without waiting.
#[cfg(unix)]
fn try_lock(lock: &File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor belongs to `lock`, open for the whole call, and `flock` only reads it.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

/// Only the tests build the queue off Unix, in one process, so there is no other owner to keep out.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature of the Unix `try_lock`, which can fail"
)]
fn try_lock(_lock: &File) -> io::Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Event, Message, Token};

    fn web_push_token(endpoint: &str) -> Entry {
        Entry::WebPushToken {
            endpoint: endpoint.to_owned(),
            p256dh: [4; 65],
            auth: [7; 16],
        }
    }

    fn message(payload: &[u8], started_app: bool) -> Entry {
        Entry::Message(Message {
            payload: payload.to_vec(),
            started_app,
        })
    }

    fn encoded(entries: &[Entry]) -> Vec<u8> {
        let mut bytes = HEADER.to_vec();
        for entry in entries {
            entry.encode(&mut bytes).unwrap();
        }
        bytes
    }

    #[test]
    fn entries_round_trip_in_order() {
        let entries = [
            Entry::FcmToken("token-1".to_owned()),
            message(br#"{"seq":"1"}"#, true),
            Entry::MessagesDropped,
            web_push_token("https://ntfy.sh/up123?up=1"),
            Entry::RegistrationFailed("unregistered by the distributor".to_owned()),
            message(b"", false),
        ];
        assert_eq!(decode(&encoded(&entries)), entries);
    }

    #[test]
    fn a_record_cut_short_by_a_killed_process_is_dropped_and_the_rest_kept() {
        let complete = [Entry::FcmToken("t".to_owned()), message(b"abc", false)];
        let unfinished = [
            message(b"never fully written", true),
            web_push_token("https://push.example/never-fully-written"),
            Entry::RegistrationFailed("never fully written".to_owned()),
        ];
        for last in unfinished {
            let mut bytes = encoded(&complete);
            let full = bytes.len();
            last.encode(&mut bytes).unwrap();
            for cut in full..bytes.len() {
                assert_eq!(decode(&bytes[..cut]), complete, "{last:?} cut at {cut}");
            }
        }
    }

    #[test]
    fn a_file_without_the_header_holds_no_entries() {
        assert_eq!(decode(b""), []);
        assert_eq!(decode(b"something else entirely"), []);
        let mut other_version = encoded(&[Entry::MessagesDropped]);
        *other_version.get_mut(HEADER.len() - 1).unwrap() ^= 0xff;
        assert_eq!(decode(&other_version), []);
    }

    #[test]
    fn an_unknown_record_ends_the_readable_part() {
        let mut bytes = encoded(&[Entry::MessagesDropped]);
        bytes.push(0xee);
        Entry::MessagesDropped.encode(&mut bytes).unwrap();
        assert_eq!(decode(&bytes), [Entry::MessagesDropped]);
    }

    #[test]
    fn a_token_that_is_not_utf8_ends_the_readable_part() {
        let mut bytes = encoded(&[Entry::MessagesDropped]);
        Entry::FcmToken("xx".to_owned()).encode(&mut bytes).unwrap();
        let last = bytes.len() - 1;
        bytes[last] = 0xff;
        assert_eq!(decode(&bytes), [Entry::MessagesDropped]);
    }

    #[test]
    fn entries_become_the_matching_events() {
        assert_eq!(
            Entry::FcmToken("t".to_owned()).into_event(),
            Event::Token(Token::Fcm("t".to_owned()))
        );
        assert_eq!(
            message(b"p", true).into_event(),
            Event::Message(Message {
                payload: b"p".to_vec(),
                started_app: true
            })
        );
        assert_eq!(Entry::MessagesDropped.into_event(), Event::MessagesDropped);
        assert_eq!(
            web_push_token("https://ntfy.sh/up1?up=1").into_event(),
            Event::Token(Token::WebPush {
                endpoint: "https://ntfy.sh/up1?up=1".to_owned(),
                p256dh: [4; 65],
                auth: [7; 16],
                expires: None,
            })
        );
        assert_eq!(
            Entry::RegistrationFailed("gone".to_owned()).into_event(),
            Event::RegistrationFailed(Error::Platform("gone".to_owned()))
        );
    }

    #[test]
    fn the_file_keeps_appended_entries_until_taken() {
        let directory = tempfile::tempdir().unwrap();
        let queue = QueueFile::new(directory.path().join("queue"));
        assert_eq!(queue.take_all().unwrap(), []);
        queue.append(&Entry::FcmToken("a".to_owned())).unwrap();
        queue.append(&message(b"b", true)).unwrap();
        // The next process to own the file finds the entries.
        drop(queue);
        let reopened = QueueFile::new(directory.path().join("queue"));
        assert_eq!(
            reopened.take_all().unwrap(),
            [Entry::FcmToken("a".to_owned()), message(b"b", true)]
        );
        assert_eq!(reopened.take_all().unwrap(), []);
        reopened.append(&Entry::MessagesDropped).unwrap();
        assert_eq!(reopened.take_all().unwrap(), [Entry::MessagesDropped]);
    }

    #[test]
    fn an_append_after_a_header_cut_short_is_kept() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue");
        for cut in 1..HEADER.len() {
            std::fs::write(&path, &HEADER[..cut]).unwrap();
            let queue = QueueFile::new(path.clone());
            queue.append(&Entry::MessagesDropped).unwrap();
            assert_eq!(
                queue.take_all().unwrap(),
                [Entry::MessagesDropped],
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn an_append_after_a_record_cut_short_is_kept_with_the_entries_before_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue");
        let mut unfinished = Vec::new();
        Entry::FcmToken("never fully written".to_owned())
            .encode(&mut unfinished)
            .unwrap();
        for cut in 1..unfinished.len() {
            let mut bytes = encoded(&[Entry::MessagesDropped]);
            bytes.extend_from_slice(&unfinished[..cut]);
            std::fs::write(&path, &bytes).unwrap();
            let queue = QueueFile::new(path.clone());
            queue.append(&Entry::FcmToken("later".to_owned())).unwrap();
            assert_eq!(
                queue.take_all().unwrap(),
                [Entry::MessagesDropped, Entry::FcmToken("later".to_owned())],
                "cut at {cut}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_second_owner_waits_for_the_first_and_then_takes_its_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue");
        let first = QueueFile::new(path.clone());
        first.append(&Entry::FcmToken("first".to_owned())).unwrap();
        let second = QueueFile::new(path);
        assert!(
            second.claim_within(Duration::from_millis(200)).is_err(),
            "a second owner must not write while the first owns the queue"
        );
        drop(first);
        second.claim_within(Duration::from_millis(200)).unwrap();
        second.append(&Entry::MessagesDropped).unwrap();
        assert_eq!(
            second.take_all().unwrap(),
            [Entry::FcmToken("first".to_owned()), Entry::MessagesDropped]
        );
    }
}
