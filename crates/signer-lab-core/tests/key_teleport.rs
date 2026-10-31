use std::{collections::HashMap, path::PathBuf, str::FromStr as _, sync::Mutex};

use signer_lab_core::{
    KeyTeleportError, KeyTeleportRequest, KeyTeleportResponse, KeyTeleportSessionStore, LabStore,
    MnemonicSecret, NumericCode, SecretStore, SecretStoreError, SignerId, SignerRepository,
    TeleportPassword,
};

#[derive(Default)]
struct Secrets(Mutex<HashMap<SignerId, String>>);

impl SecretStore for Secrets {
    fn save(&self, id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .expect("lock")
            .insert(id, secret.expose_phrase().to_owned());
        Ok(())
    }

    fn load(&self, id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
        MnemonicSecret::parse(self.0.lock().expect("lock").get(&id).expect("secret"))
            .map_err(SecretStoreError::InvalidSecret)
    }

    fn delete(&self, id: SignerId) -> Result<(), SecretStoreError> {
        self.0.lock().expect("lock").remove(&id);
        Ok(())
    }
}

/// One Signer Lab data directory, standing in for one device
struct Device {
    repository: SignerRepository<Secrets>,
    sessions: KeyTeleportSessionStore,
    root: PathBuf,
}

impl Device {
    async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("signer-lab-teleport-{}", uuid::Uuid::new_v4()));
        let store = LabStore::create(root.join("metadata.db"))
            .await
            .expect("store");

        Self {
            repository: SignerRepository::new(store, Secrets::default(), root.join("wallets")),
            sessions: KeyTeleportSessionStore::new(&root),
            root,
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn teleported_signer_is_imported_once_from_scanned_text() {
    let mut sender = Device::new().await;
    let mut receiver = Device::new().await;
    let created = sender
        .repository
        .generate("Sender", 0, 24)
        .await
        .expect("sender signer");

    let request = receiver
        .sessions
        .start()
        .expect("start")
        .request()
        .expect("request");
    let request_text = request.request.qr().expect("request QR").bbqr;
    let code = request.numeric_code.to_string();

    let secret = sender
        .repository
        .reveal_secret(created.signer.id)
        .expect("secret");
    let sent = KeyTeleportRequest::from_str(&request_text)
        .expect("scan request")
        .send_mnemonic(&secret, &NumericCode::from_str(&code).expect("code"))
        .expect("send");

    let response =
        KeyTeleportResponse::from_str(&sent.response.qr().expect("QR").url).expect("scan response");
    let password = TeleportPassword::from_str(&sent.password.grouped()).expect("password");
    let imported = receiver
        .sessions
        .import(
            &mut receiver.repository,
            "Received",
            0,
            &response,
            &password,
        )
        .await
        .expect("import");

    assert_eq!(imported.fingerprint, created.signer.fingerprint);
    assert_eq!(
        receiver
            .repository
            .reveal_secret(imported.id)
            .expect("received secret")
            .expose_phrase(),
        created.secret.expose_phrase()
    );

    // the finished session cannot decrypt a replayed response
    assert!(matches!(
        receiver
            .sessions
            .import(&mut receiver.repository, "Replay", 0, &response, &password)
            .await,
        Err(KeyTeleportError::NoReceiveSession)
    ));
    assert_eq!(
        receiver.repository.signers().await.expect("signers").len(),
        1
    );
}

#[tokio::test]
async fn failed_import_keeps_the_session_for_a_retry() {
    let mut sender = Device::new().await;
    let mut receiver = Device::new().await;
    let created = sender
        .repository
        .generate("Sender", 0, 12)
        .await
        .expect("sender signer");

    let request = receiver
        .sessions
        .start()
        .expect("start")
        .request()
        .expect("request");
    let sent = request
        .request
        .send_mnemonic(&created.secret, &request.numeric_code)
        .expect("send");

    assert!(matches!(
        receiver
            .sessions
            .import(
                &mut receiver.repository,
                "Received",
                0,
                &sent.response,
                &TeleportPassword::from_bytes([0; 5]),
            )
            .await,
        Err(KeyTeleportError::WrongPassword)
    ));

    let imported = receiver
        .sessions
        .import(
            &mut receiver.repository,
            "Received",
            0,
            &sent.response,
            &sent.password,
        )
        .await
        .expect("retry");
    assert_eq!(imported.fingerprint, created.signer.fingerprint);
}
