//! The file where Android keeps events until a UI handler takes them.
//!
//! The file starts with [`HEADER`] and holds records appended one by one. A record is a tag
//! byte followed by its fields, and lengths are little-endian `u32`.
//!
//! | Tag | Entry | Fields |
//! |---|---|---|
//! | 1 | message | `started_app` (0 or 1), payload length, payload |
//! | 2 | FCM token | length, UTF-8 token |
//! | 3 | messages dropped | none |
//!
//! A process killed mid-append leaves a short last record, which reading ignores.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

use crate::{Event, Message, Token};

/// Names the format and its version.
pub(crate) const HEADER: &[u8; 8] = b"PUSHUPS\x01";

const MESSAGE: u8 = 1;
const FCM_TOKEN: u8 = 2;
const MESSAGES_DROPPED: u8 = 3;

/// An event waiting in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Message(Message),
    FcmToken(String),
    MessagesDropped,
}

impl Entry {
    pub(crate) fn into_event(self) -> Event {
        match self {
            Self::Message(message) => Event::Message(message),
            Self::FcmToken(token) => Event::Token(Token::Fcm(token)),
            Self::MessagesDropped => Event::MessagesDropped,
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
    while let Some((entry, after)) = decode_one(rest) {
        entries.push(entry);
        rest = after;
    }
    entries
}

fn decode_one(bytes: &[u8]) -> Option<(Entry, &[u8])> {
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
            let message = Message {
                payload: payload.to_vec(),
                started_app,
            };
            Some((Entry::Message(message), rest))
        }
        FCM_TOKEN => {
            let (token, rest) = take_bytes(rest)?;
            let token = String::from_utf8(token.to_vec()).ok()?;
            Some((Entry::FcmToken(token), rest))
        }
        MESSAGES_DROPPED => Some((Entry::MessagesDropped, rest)),
        _ => None,
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

/// The queue file at a fixed path.
#[derive(Debug)]
pub(crate) struct QueueFile {
    path: PathBuf,
}

impl QueueFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Appends `entry` and waits until it is on storage.
    pub(crate) fn append(&self, entry: &Entry) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        let mut record = Vec::new();
        if !starts_with_header(&mut file)? {
            // A first append cut inside the header leaves a prefix no reader accepts.
            file.set_len(0)?;
            record.extend_from_slice(HEADER);
        }
        entry.encode(&mut record)?;
        file.write_all(&record)?;
        file.sync_data()
    }

    /// Returns every entry in append order and empties the file.
    pub(crate) fn take_all(&self) -> io::Result<Vec<Entry>> {
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

fn starts_with_header(file: &mut File) -> io::Result<bool> {
    let mut start = [0; HEADER.len()];
    match file.read_exact(&mut start) {
        Ok(()) => Ok(&start == HEADER),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, Message, Token};

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
            message(b"", false),
        ];
        assert_eq!(decode(&encoded(&entries)), entries);
    }

    #[test]
    fn a_record_cut_short_by_a_killed_process_is_dropped_and_the_rest_kept() {
        let complete = [Entry::FcmToken("t".to_owned()), message(b"abc", false)];
        let mut bytes = encoded(&complete);
        let full = bytes.len();
        message(b"never fully written", true)
            .encode(&mut bytes)
            .unwrap();
        for cut in full..bytes.len() {
            assert_eq!(decode(&bytes[..cut]), complete, "cut at {cut}");
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
    }

    #[test]
    fn the_file_keeps_appended_entries_until_taken() {
        let directory = tempfile::tempdir().unwrap();
        let queue = QueueFile::new(directory.path().join("queue"));
        assert_eq!(queue.take_all().unwrap(), []);
        queue.append(&Entry::FcmToken("a".to_owned())).unwrap();
        queue.append(&message(b"b", true)).unwrap();
        let reopened = QueueFile::new(directory.path().join("queue"));
        assert_eq!(
            reopened.take_all().unwrap(),
            [Entry::FcmToken("a".to_owned()), message(b"b", true)]
        );
        assert_eq!(reopened.take_all().unwrap(), []);
        reopened.append(&Entry::MessagesDropped).unwrap();
        assert_eq!(queue.take_all().unwrap(), [Entry::MessagesDropped]);
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
}
