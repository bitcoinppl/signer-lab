use std::{
    fmt,
    fs::{self, DirBuilder, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
    str::FromStr,
};

use jiff::{SignedDuration, Timestamp};
use keyteleport::{
    DecodedPayload, Packet, Payload, ReceiverPacket, ReceiverSession, SenderPacket, SenderSession,
    UnsupportedPayloadKind,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub use keyteleport::{NumericCode, TeleportPassword};

use crate::{MnemonicSecret, RepositoryError, SecretStore, SignerRepository, VirtualSigner};

// matches Cove so a forgotten request cannot decrypt a transfer made much later
const RECEIVE_SESSION_TTL: SignedDuration = SignedDuration::from_hours(24);

/// The packet kinds that can appear in Key Teleport QR or link text
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyTeleportPacketKind {
    /// A receiver request that a sender encrypts a secret for
    Receiver,
    /// An encrypted secret returned by a sender
    Sender,
    /// A multisig PSBT transfer, which Signer Lab does not support
    Psbt,
}

impl fmt::Display for KeyTeleportPacketKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Receiver => "receiver request",
            Self::Sender => "sender response",
            Self::Psbt => "PSBT",
        })
    }
}

/// A single-part QR payload and the equivalent `keyteleport.com` link
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyTeleportQr {
    /// Base32 BBQR text to render as one QR code
    pub bbqr: String,
    /// Link that carries the same packet in its fragment
    pub url: String,
}

/// A receiver request scanned from another device
#[derive(Clone, PartialEq, Eq)]
pub struct KeyTeleportRequest(ReceiverPacket);

impl KeyTeleportRequest {
    /// Encrypts recovery words for this receiver
    ///
    /// The returned password must reach the receiver separately from the response QR
    ///
    /// # Errors
    ///
    /// Returns an error when the receiver code does not match this request or the mnemonic has a
    /// word count that Key Teleport cannot carry
    pub fn send_mnemonic(
        &self,
        secret: &MnemonicSecret,
        code: &NumericCode,
    ) -> Result<SentKeyTeleport, KeyTeleportError> {
        let mnemonic = secret.to_mnemonic().map_err(KeyTeleportError::Mnemonic)?;
        let payload = Payload::mnemonic(mnemonic).map_err(KeyTeleportError::UnsupportedMnemonic)?;

        // about half of wrong codes decrypt to an invalid key and fail here; the rest encrypt to an
        // unknown key, which the receiver reports as a session mismatch
        let sender = SenderSession::new(&self.0, code).map_err(KeyTeleportError::ReceiverCode)?;
        let response = sender.send(payload).map_err(KeyTeleportError::Encrypt)?;

        Ok(SentKeyTeleport {
            response: KeyTeleportResponse(response.packet),
            password: response.password,
        })
    }

    /// Encodes the request for display
    ///
    /// # Errors
    ///
    /// Returns an error if the packet cannot be encoded as single-part BBQR
    pub fn qr(&self) -> Result<KeyTeleportQr, KeyTeleportError> {
        encode(&Packet::Receiver(self.0.clone()))
    }
}

impl FromStr for KeyTeleportRequest {
    type Err = KeyTeleportError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match parse_packet(value)? {
            Packet::Receiver(packet) => Ok(Self(packet)),
            other => Err(unexpected(KeyTeleportPacketKind::Receiver, &other)),
        }
    }
}

impl fmt::Debug for KeyTeleportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyTeleportRequest")
    }
}

/// An encrypted secret returned by a sender
#[derive(Clone, PartialEq, Eq)]
pub struct KeyTeleportResponse(SenderPacket);

impl KeyTeleportResponse {
    /// Encodes the response for display
    ///
    /// # Errors
    ///
    /// Returns an error if the packet cannot be encoded as single-part BBQR
    pub fn qr(&self) -> Result<KeyTeleportQr, KeyTeleportError> {
        encode(&Packet::Sender(self.0.clone()))
    }
}

impl FromStr for KeyTeleportResponse {
    type Err = KeyTeleportError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match parse_packet(value)? {
            Packet::Sender(packet) => Ok(Self(packet)),
            other => Err(unexpected(KeyTeleportPacketKind::Sender, &other)),
        }
    }
}

impl fmt::Debug for KeyTeleportResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyTeleportResponse")
    }
}

/// Recovery words encrypted for one receiver request
#[derive(Debug)]
pub struct SentKeyTeleport {
    /// Response QR for the receiver to scan
    pub response: KeyTeleportResponse,
    /// Password the receiver types after scanning the response
    pub password: TeleportPassword,
}

/// The receiver request that a sender scans, plus the code it needs
#[derive(Debug, Clone)]
pub struct KeyTeleportReceiveRequest {
    /// Request QR for the sender to scan
    pub request: KeyTeleportRequest,
    /// Eight-digit code the sender types after scanning the request
    pub numeric_code: NumericCode,
    /// Time after which the session is discarded
    pub expires_at: Timestamp,
}

/// A receive session waiting for a sender response
#[derive(Debug)]
pub struct KeyTeleportReceiveSession {
    receiver: ReceiverSession,
    created_at: Timestamp,
}

impl KeyTeleportReceiveSession {
    /// Returns the request to show the sender
    ///
    /// The request is derived from the session key, so it is identical every time
    ///
    /// # Errors
    ///
    /// Returns an error if the protocol cannot build the receiver packet
    pub fn request(&self) -> Result<KeyTeleportReceiveRequest, KeyTeleportError> {
        let request = self.receiver.request().map_err(KeyTeleportError::Encode)?;

        Ok(KeyTeleportReceiveRequest {
            request: KeyTeleportRequest(request.packet),
            numeric_code: request.numeric_code,
            expires_at: self.expires_at(),
        })
    }

    /// Time the session was started
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Time after which the session is discarded
    #[must_use]
    pub fn expires_at(&self) -> Timestamp {
        self.created_at
            .checked_add(RECEIVE_SESSION_TTL)
            .unwrap_or(Timestamp::MAX)
    }

    /// Decrypts recovery words from a sender response
    ///
    /// # Errors
    ///
    /// Returns an error when the response was made for another session, the password is wrong, or
    /// the payload is not a BIP39 mnemonic
    pub fn decrypt_mnemonic(
        &self,
        response: &KeyTeleportResponse,
        password: &TeleportPassword,
    ) -> Result<MnemonicSecret, KeyTeleportError> {
        // both layers end with a checksum, so a mismatch identifies which input was wrong
        let pending = self
            .receiver
            .decode_step1(&response.0)
            .map_err(|error| match error {
                keyteleport::Error::Checksum => KeyTeleportError::SessionMismatch,
                error => KeyTeleportError::Decode(error),
            })?;

        let payload = pending.complete(password).map_err(|error| match error {
            keyteleport::Error::Checksum => KeyTeleportError::WrongPassword,
            keyteleport::Error::UnsupportedPayload(kind) => {
                KeyTeleportError::UnsupportedPayload(UnsupportedKeyTeleportPayload::Other(kind))
            }
            error => KeyTeleportError::Decode(error),
        })?;

        match payload {
            DecodedPayload::Mnemonic(mnemonic) => {
                MnemonicSecret::parse(&Zeroizing::new(mnemonic.to_string()))
                    .map_err(KeyTeleportError::Mnemonic)
            }
            DecodedPayload::Xprv(_) => Err(KeyTeleportError::UnsupportedPayload(
                UnsupportedKeyTeleportPayload::Xprv,
            )),
            DecodedPayload::Notes(_) => Err(KeyTeleportError::UnsupportedPayload(
                UnsupportedKeyTeleportPayload::SecureNotes,
            )),
        }
    }

    fn is_expired(&self, now: Timestamp) -> bool {
        now >= self.expires_at()
    }
}

/// Saves the receiver key between showing a request and scanning the response
///
/// The CLI finishes a receive in a later process, and the app may restart while the user works on
/// the sending device, so the session lives in an owner-only file next to other test secrets
#[derive(Debug, Clone)]
pub struct KeyTeleportSessionStore(PathBuf);

impl KeyTeleportSessionStore {
    /// Creates a store for the receive session under the application data directory
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self(data_dir.join("key-teleport").join("receive-session.json"))
    }

    /// File that holds the receive session
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Starts a receive session, replacing any earlier one
    ///
    /// # Errors
    ///
    /// Returns an error when the session cannot be saved
    pub fn start(&self) -> Result<KeyTeleportReceiveSession, KeyTeleportError> {
        let session = KeyTeleportReceiveSession {
            receiver: ReceiverSession::new(),
            created_at: Timestamp::now(),
        };
        self.save(&session)?;

        Ok(session)
    }

    /// Loads the current receive session
    ///
    /// Expired sessions are deleted and reported as missing
    ///
    /// # Errors
    ///
    /// Returns an error when the session file cannot be read, parsed, or deleted
    pub fn active(&self) -> Result<Option<KeyTeleportReceiveSession>, KeyTeleportError> {
        let Some(session) = self.load()? else {
            return Ok(None);
        };

        if session.is_expired(Timestamp::now()) {
            self.cancel()?;
            return Ok(None);
        }

        Ok(Some(session))
    }

    /// Deletes the receive session
    ///
    /// A missing session is treated as already deleted
    ///
    /// # Errors
    ///
    /// Returns an error when the session file cannot be deleted
    pub fn cancel(&self) -> Result<(), KeyTeleportError> {
        match fs::remove_file(&self.0) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(self.io_error(source)),
        }
    }

    /// Decrypts a sender response, imports its recovery words, and ends the session
    ///
    /// The session survives a failed import so the same response can be retried
    ///
    /// # Errors
    ///
    /// Returns an error when no session is active, decryption fails, or the signer cannot be saved
    pub async fn import<S: SecretStore>(
        &self,
        repository: &mut SignerRepository<S>,
        name: impl Into<String>,
        account_index: u32,
        response: &KeyTeleportResponse,
        password: &TeleportPassword,
    ) -> Result<VirtualSigner, KeyTeleportError> {
        let session = self.active()?.ok_or(KeyTeleportError::NoReceiveSession)?;
        let secret = session.decrypt_mnemonic(response, password)?;
        let signer = repository
            .import_secret(name, account_index, secret)
            .await
            .map_err(|error| KeyTeleportError::Repository(Box::new(error)))?;

        // the signer is already saved, and a leftover session only lives until it expires
        if let Err(error) = self.cancel() {
            tracing::warn!("cannot delete finished Key Teleport receive session: {error}");
        }

        Ok(signer)
    }

    fn load(&self) -> Result<Option<KeyTeleportReceiveSession>, KeyTeleportError> {
        let contents = match fs::read(&self.0) {
            Ok(contents) => Zeroizing::new(contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(self.io_error(source)),
        };

        let persisted: PersistedSession =
            serde_json::from_slice(&contents).map_err(KeyTeleportError::CorruptSession)?;
        let receiver = ReceiverSession::from_private_key_bytes(persisted.private_key)
            .map_err(KeyTeleportError::CorruptSessionKey)?;

        Ok(Some(KeyTeleportReceiveSession {
            receiver,
            created_at: persisted.created_at,
        }))
    }

    fn save(&self, session: &KeyTeleportReceiveSession) -> Result<(), KeyTeleportError> {
        let persisted = PersistedSession {
            private_key: session.receiver.private_key_bytes(),
            created_at: session.created_at,
        };
        let contents = Zeroizing::new(
            serde_json::to_vec(&persisted).map_err(KeyTeleportError::CorruptSession)?,
        );

        if let Some(directory) = self.0.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)
                .map_err(|source| self.io_error(source))?;
        }

        // write a sibling and rename it so a crash never leaves a truncated key
        let staged = self.0.with_extension("json.tmp");
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&staged)
            .and_then(|mut file| {
                file.write_all(&contents)?;
                file.sync_all()
            })
            .map_err(|source| self.io_error(source))?;

        fs::rename(&staged, &self.0).map_err(|source| self.io_error(source))
    }

    fn io_error(&self, source: io::Error) -> KeyTeleportError {
        KeyTeleportError::SessionIo {
            path: self.0.clone(),
            source,
        }
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct PersistedSession {
    private_key: [u8; 32],
    #[zeroize(skip)]
    created_at: Timestamp,
}

/// A decrypted payload that Signer Lab cannot store as a virtual signer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedKeyTeleportPayload {
    /// A BIP32 master extended private key
    Xprv,
    /// COLDCARD Secure Notes & Passwords
    SecureNotes,
    /// A payload type that the protocol crate recognizes but cannot decode
    Other(UnsupportedPayloadKind),
}

impl fmt::Display for UnsupportedKeyTeleportPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xprv => formatter.write_str("an extended private key"),
            Self::SecureNotes => formatter.write_str("secure notes"),
            Self::Other(kind) => write!(formatter, "payload type {kind}"),
        }
    }
}

/// Errors from Key Teleport transfers
#[derive(Debug, thiserror::Error)]
pub enum KeyTeleportError {
    /// The text is not a Key Teleport QR or link
    #[error("not a Key Teleport QR or keyteleport.com link: {0}")]
    InvalidPacket(#[source] keyteleport::Error),
    /// The text holds a different packet kind than the step expects
    #[error("expected a Key Teleport {expected}, found a {found}")]
    UnexpectedPacket {
        /// Packet kind the step needs
        expected: KeyTeleportPacketKind,
        /// Packet kind in the scanned text
        found: KeyTeleportPacketKind,
    },
    /// The receiver code decrypts the request to an invalid public key
    ///
    /// Some wrong codes still yield a valid key, so this error does not catch every typo
    #[error("receiver code does not match the receiver QR")]
    ReceiverCode(#[source] keyteleport::Error),
    /// The mnemonic cannot be carried by Key Teleport
    #[error("{0}")]
    UnsupportedMnemonic(#[source] keyteleport::Error),
    /// Encrypting the payload failed
    #[error("cannot encrypt Key Teleport payload: {0}")]
    Encrypt(#[source] keyteleport::Error),
    /// Encoding a packet failed
    #[error("cannot encode Key Teleport packet: {0}")]
    Encode(#[source] keyteleport::Error),
    /// The response was encrypted for another receive session or with a wrong receiver code
    #[error(
        "sender QR was not made for the current receive session; check that the sender used the latest request and receiver code"
    )]
    SessionMismatch,
    /// The teleport password is wrong
    #[error("teleport password is incorrect")]
    WrongPassword,
    /// The response decrypted to a payload that is not recovery words
    #[error("Key Teleport sent {0}; Signer Lab imports only BIP39 recovery words")]
    UnsupportedPayload(UnsupportedKeyTeleportPayload),
    /// The decrypted payload is malformed
    #[error("cannot decode Key Teleport payload: {0}")]
    Decode(#[source] keyteleport::Error),
    /// No unexpired receive session exists
    #[error("no active Key Teleport receive session; start a new receive")]
    NoReceiveSession,
    /// Reading or writing the session file failed
    #[error("Key Teleport session file operation failed at {}: {source}", path.display())]
    SessionIo {
        /// Session file
        path: PathBuf,
        /// Filesystem failure
        #[source]
        source: io::Error,
    },
    /// The session file is not valid session JSON
    #[error("Key Teleport receive session is corrupt: {0}")]
    CorruptSession(#[source] serde_json::Error),
    /// The session file holds an invalid private key
    #[error("Key Teleport receive session key is invalid: {0}")]
    CorruptSessionKey(#[source] keyteleport::Error),
    /// The mnemonic is invalid
    #[error("{0}")]
    Mnemonic(#[source] crate::MnemonicError),
    /// Saving the received signer failed
    #[error("{0}")]
    Repository(#[source] Box<RepositoryError>),
}

fn parse_packet(value: &str) -> Result<Packet, KeyTeleportError> {
    Packet::from_url(value).map_err(KeyTeleportError::InvalidPacket)
}

fn unexpected(expected: KeyTeleportPacketKind, found: &Packet) -> KeyTeleportError {
    let found = match found {
        Packet::Receiver(_) => KeyTeleportPacketKind::Receiver,
        Packet::Sender(_) => KeyTeleportPacketKind::Sender,
        Packet::Psbt(_) => KeyTeleportPacketKind::Psbt,
    };

    KeyTeleportError::UnexpectedPacket { expected, found }
}

fn encode(packet: &Packet) -> Result<KeyTeleportQr, KeyTeleportError> {
    Ok(KeyTeleportQr {
        bbqr: packet.to_bbqr_part().map_err(KeyTeleportError::Encode)?,
        url: packet.to_url().map_err(KeyTeleportError::Encode)?,
    })
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt as _, str::FromStr as _};

    use jiff::{SignedDuration, Timestamp};

    use super::{
        KeyTeleportError, KeyTeleportPacketKind, KeyTeleportRequest, KeyTeleportResponse,
        KeyTeleportSessionStore, NumericCode, TeleportPassword, UnsupportedKeyTeleportPayload,
    };
    use crate::{MnemonicSecret, SignerId};

    const WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn store() -> (KeyTeleportSessionStore, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("signer-lab-key-teleport-{}", SignerId::new()));

        (KeyTeleportSessionStore::new(&directory), directory)
    }

    fn secret() -> MnemonicSecret {
        MnemonicSecret::parse(WORDS).expect("test mnemonic")
    }

    #[test]
    fn mnemonic_round_trips_through_scanned_text_and_a_reloaded_session() {
        let (store, directory) = store();
        let started = store.start().expect("start");
        let request = started.request().expect("request");
        let request_qr = request.request.qr().expect("request QR");

        // the sender only sees the QR text and the code the receiver reads aloud
        let scanned = KeyTeleportRequest::from_str(&request_qr.url).expect("scan request link");
        let code = NumericCode::from_str(&request.numeric_code.grouped()).expect("code");
        let sent = scanned.send_mnemonic(&secret(), &code).expect("send");
        let response_qr = sent.response.qr().expect("response QR");

        // reload from disk as the CLI does in its finishing process
        let reloaded = store.active().expect("load").expect("active session");
        assert_eq!(
            reloaded
                .request()
                .expect("request")
                .request
                .qr()
                .expect("QR"),
            request_qr
        );

        let response = KeyTeleportResponse::from_str(&response_qr.bbqr).expect("scan response");
        let password =
            TeleportPassword::from_str(&sent.password.grouped().to_lowercase()).expect("password");
        let received = reloaded
            .decrypt_mnemonic(&response, &password)
            .expect("decrypt");
        assert_eq!(received.expose_phrase(), WORDS);

        let mode = std::fs::metadata(store.path())
            .expect("session file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn wrong_inputs_report_which_step_failed() {
        let (store, directory) = store();
        let session = store.start().expect("start");
        let request = session.request().expect("request");
        let sent = request
            .request
            .send_mnemonic(&secret(), &request.numeric_code)
            .expect("send");

        let wrong_password = TeleportPassword::from_bytes([0; 5]);
        assert!(matches!(
            session.decrypt_mnemonic(&sent.response, &wrong_password),
            Err(KeyTeleportError::WrongPassword)
        ));

        // a newer session replaces the key the sender encrypted for
        let replaced = store.start().expect("restart");
        assert!(matches!(
            replaced.decrypt_mnemonic(&sent.response, &sent.password),
            Err(KeyTeleportError::SessionMismatch)
        ));

        let response_text = sent.response.qr().expect("QR").bbqr;
        assert!(matches!(
            KeyTeleportRequest::from_str(&response_text),
            Err(KeyTeleportError::UnexpectedPacket {
                expected: KeyTeleportPacketKind::Receiver,
                found: KeyTeleportPacketKind::Sender,
            })
        ));

        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn xprv_payloads_are_rejected_instead_of_imported() {
        let (store, directory) = store();
        let session = store.start().expect("start");
        let request = session.request().expect("request");
        let sender = keyteleport::SenderSession::new(&request.request.0, &request.numeric_code)
            .expect("sender");
        let xprv = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
        let response = sender
            .send(keyteleport::Payload::xprv(xprv).expect("xprv payload"))
            .expect("send");

        assert!(matches!(
            session.decrypt_mnemonic(&KeyTeleportResponse(response.packet), &response.password),
            Err(KeyTeleportError::UnsupportedPayload(
                UnsupportedKeyTeleportPayload::Xprv
            ))
        ));

        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn expired_sessions_are_deleted() {
        let (store, directory) = store();
        let mut session = store.start().expect("start");
        session.created_at = Timestamp::now() - SignedDuration::from_hours(25);
        store.save(&session).expect("save expired");

        assert!(store.active().expect("load").is_none());
        assert!(!store.path().exists());

        store.cancel().expect("cancel is idempotent");
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
